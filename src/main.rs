//! CustomAIO - fan/pump profiles and an LCD readout for NZXT Kraken coolers.
//!
//! One binary, named `fan`, so the everyday commands read naturally:
//!
//!     fan silent      fan perf      fan status
//!     fan lcd         fan preview   fan devices
//!
//! `fan lcd` is the long-running service; everything else is a one-shot.

// Keep the console window from flashing when Task Scheduler starts the
// service, while still behaving like a console app when launched from a
// terminal. `fan lcd --quiet` is what the scheduled task uses.
mod config;
mod kraken;
mod render;
mod sensors;

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("help");
    let rest = &args[args.len().min(1)..];

    let result = match command {
        "silent" => apply_profile("silent"),
        "perf" | "performance" => apply_profile("performance"),
        "status" => status(),
        "lcd" | "service" => service(rest),
        "preview" => preview(rest),
        "devices" | "list" => devices(),
        "help" | "-h" | "--help" => {
            help();
            Ok(())
        }
        other => Err(format!("unknown command '{other}'. Try `fan help`.")),
    };

    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{RED}Error:{RESET} {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

// ============================================================
// Commands
// ============================================================

fn apply_profile(name: &str) -> Result<(), String> {
    let cfg = config::Config::load()?;
    if !cfg.fan.enabled {
        return Err("fan control is disabled in config.toml ([fan] enabled = false)".into());
    }
    let profile = match name {
        "silent" => &cfg.fan.silent,
        _ => &cfg.fan.performance,
    };

    header(&format!("Applying {name} profile"));
    let device = kraken::Kraken::open(&cfg.device)?;
    println!("  {GRAY}{}{RESET}", device.model.name);

    // Pump first: it should already be ramping before the fans follow.
    device.set_curve(kraken::Channel::Pump, &profile.pump)?;
    println!("  pump curve set");
    device.set_curve(kraken::Channel::Fan, &profile.fan)?;
    println!("  fan curve set");

    let _ = std::fs::write(data_path("profile.txt"), name);
    println!("\n{GREEN}{name} profile applied.{RESET}\n");
    Ok(())
}

fn status() -> Result<(), String> {
    let cfg = config::Config::load()?;
    header("Status");

    let last = std::fs::read_to_string(data_path("profile.txt")).unwrap_or_else(|_| "unknown".into());
    let device = kraken::Kraken::open(&cfg.device)?;
    println!("  {WHITE}Device{RESET}        {}", device.model.name);
    println!("  {WHITE}Serial{RESET}        {}", device.serial);
    println!("  {WHITE}Last profile{RESET}  {}", last.trim());

    match device.status() {
        Ok(s) => {
            println!("  {WHITE}Liquid{RESET}        {:.1} C", s.liquid_temp);
            println!("  {WHITE}Pump{RESET}          {} RPM ({}%)", s.pump_rpm, s.pump_duty);
            println!("  {WHITE}Fan{RESET}           {} RPM ({}%)", s.fan_rpm, s.fan_duty);
        }
        Err(e) => println!("  {GRAY}cooler status unavailable - {e}{RESET}"),
    }

    let (readings, notes) = sensors::Readings::open(&cfg.sensors);
    let (cpu, gpu) = readings.sample();
    println!("  {WHITE}CPU{RESET}           {}", temp_or_na(cpu));
    println!("  {WHITE}GPU{RESET}           {}", temp_or_na(gpu));
    for n in notes {
        println!("  {GRAY}{n}{RESET}");
    }
    println!();
    Ok(())
}

fn devices() -> Result<(), String> {
    header("Supported coolers detected");
    let api = hidapi::HidApi::new().map_err(|e| format!("HID init failed: {e}"))?;
    let mut found = 0;
    let mut seen: Vec<(u16, u16, String)> = Vec::new();
    for info in api.device_list() {
        let Some(model) = kraken::MODELS
            .iter()
            .find(|m| m.vid == info.vendor_id() && m.pid == info.product_id())
        else {
            continue;
        };
        let serial = info.serial_number().unwrap_or("").to_string();
        let key = (model.vid, model.pid, serial.clone());
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        found += 1;
        println!("  {WHITE}{}{RESET}", model.name);
        println!(
            "    VID 0x{:04X}  PID 0x{:04X}  serial {}",
            model.vid,
            model.pid,
            if serial.is_empty() { "not reported" } else { &serial }
        );
        println!(
            "    LCD {}x{}{}",
            model.resolution.0,
            model.resolution.1,
            if model.modern { "  (image upload not yet implemented)" } else { "" }
        );
    }
    if found == 0 {
        println!("  {GRAY}none found - check the USB header and close NZXT CAM{RESET}");
    }
    println!();
    Ok(())
}

/// Render a frame to PNG without touching any hardware. Useful for trying
/// styles and colours. `fan preview dial` overrides the configured style.
fn preview(args: &[String]) -> Result<(), String> {
    let mut cfg = config::Config::load()?;
    if let Some(style) = args.first() {
        cfg.style.name = style.clone();
    }
    let size = 320;
    let mut renderer = render::Renderer::new(&cfg.style.options, size)?;

    // Prefer live readings so the preview reflects reality, but fall back to
    // sample values when a sensor is unavailable.
    let (readings, _) = sensors::Readings::open(&cfg.sensors);
    let (cpu, gpu) = readings.sample();
    let (cpu, gpu) = (cpu.or(Some(38.0)), gpu.or(Some(52.0)));

    let pixmap = renderer.draw(&cfg.style, cpu, gpu, cfg.display.rotation);
    let out = config::resolve(&cfg.display.png_path);
    render::write_png(pixmap, &out)?;
    println!("Style '{}' rendered to {}", cfg.style.name, out.display());
    Ok(())
}

// ============================================================
// The LCD service
// ============================================================

fn service(args: &[String]) -> Result<(), String> {
    let quiet = args.iter().any(|a| a == "--quiet" || a == "-q");
    if quiet {
        // Detach from the console Task Scheduler hands us, so the service
        // leaves no window on the desktop. Staying in the user's session
        // (rather than session 0) is what keeps the GPU sensors readable.
        unsafe { windows_sys::Win32::System::Console::FreeConsole() };
    }
    let cfg = config::Config::load()?;
    let mut log = Logger::new(cfg.general.log, quiet);

    let (readings, notes) = sensors::Readings::open(&cfg.sensors);
    for n in &notes {
        log.info(n);
    }

    // The display is optional: with `lcd = false` this still runs as a
    // sensor-to-PNG service.
    let mut device = if cfg.display.lcd {
        match kraken::Kraken::open(&cfg.device) {
            Ok(d) => {
                log.info(&format!("Cooler: {}", d.model.name));
                if let Err(e) = d.set_brightness(cfg.display.brightness) {
                    log.info(&format!("Could not set brightness - {e}"));
                }
                // Frames are pre-rotated, so the panel must stay at 0.
                if let Err(e) = d.reset_orientation() {
                    log.info(&format!("Could not reset panel orientation - {e}"));
                }
                Some(d)
            }
            Err(e) => {
                log.info(&format!("LCD disabled - {e}"));
                None
            }
        }
    } else {
        None
    };

    let size = device.as_ref().map(|d| d.model.resolution.0).unwrap_or(320);
    let mut renderer = render::Renderer::new(&cfg.style.options, size)?;
    let png_path = config::resolve(&cfg.display.png_path);

    let interval = Duration::from_secs_f64(cfg.general.update_interval.max(0.2));
    log.info(&format!(
        "Rendering '{}' at {size}x{size} every {:.1}s",
        cfg.style.name,
        interval.as_secs_f64()
    ));

    // Absolute scheduling, so a slow frame doesn't make the period drift.
    let mut next = Instant::now();
    let mut last_error = String::new();
    loop {
        let (cpu, gpu) = readings.sample();
        let pixmap = renderer.draw(&cfg.style, cpu, gpu, cfg.display.rotation);

        if cfg.display.save_png {
            if let Err(e) = render::write_png(pixmap, &png_path) {
                log.once(&mut last_error, &e);
            }
        }
        if let Some(d) = device.as_mut() {
            if let Err(e) = d.set_image(pixmap.data()) {
                log.once(&mut last_error, &format!("LCD update failed - {e}"));
            } else {
                last_error.clear();
                log.tick(cpu, gpu);
            }
        } else {
            log.tick(cpu, gpu);
        }

        next += interval;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else {
            // Fell behind; resynchronise rather than trying to catch up.
            next = now;
        }
    }
}

// ============================================================
// Small helpers
// ============================================================

fn data_path(name: &str) -> PathBuf {
    let dir = config::base_dir().join("data");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(name)
}

fn temp_or_na(t: Option<f32>) -> String {
    t.map(|v| format!("{v:.1} C")).unwrap_or_else(|| "N/A".into())
}

/// Appends to data/customaio.log, trimming it when it gets large. Frame ticks
/// are only written when logging is on, so a quiet config does no disk I/O.
struct Logger {
    file: Option<std::fs::File>,
    console: bool,
}

impl Logger {
    fn new(enabled: bool, quiet: bool) -> Self {
        let file = if enabled {
            let path = {
                let dir = config::base_dir().join("data");
                let _ = std::fs::create_dir_all(&dir);
                dir.join("customaio.log")
            };
            // Start fresh once the log passes half a megabyte.
            if std::fs::metadata(&path).map(|m| m.len() > 512_000).unwrap_or(false) {
                let _ = std::fs::remove_file(&path);
            }
            std::fs::OpenOptions::new().create(true).append(true).open(&path).ok()
        } else {
            None
        };
        Logger { file, console: !quiet }
    }

    fn write(&mut self, line: &str) {
        if self.console {
            println!("{line}");
        }
        if let Some(f) = self.file.as_mut() {
            let _ = writeln!(f, "{line}");
        }
    }

    fn info(&mut self, msg: &str) {
        self.write(msg);
    }

    fn tick(&mut self, cpu: Option<f32>, gpu: Option<f32>) {
        if self.file.is_none() && !self.console {
            return;
        }
        self.write(&format!("CPU {} | GPU {}", temp_or_na(cpu), temp_or_na(gpu)));
    }

    /// Log a recurring failure only when the message changes, so a disconnected
    /// cooler doesn't fill the log with one line per second.
    fn once(&mut self, last: &mut String, msg: &str) {
        if last != msg {
            *last = msg.to_string();
            self.write(msg);
        }
    }
}

const RESET: &str = "\x1b[0m";
const GREEN: &str = "\x1b[92m";
const CYAN: &str = "\x1b[96m";
const WHITE: &str = "\x1b[97m";
const GRAY: &str = "\x1b[90m";
const RED: &str = "\x1b[91m";
const ORANGE: &str = "\x1b[38;5;208m";
const YELLOW: &str = "\x1b[93m";

fn header(title: &str) {
    println!("\n{CYAN}========================================{RESET}");
    println!("{WHITE}             CustomAIO{RESET}");
    println!("{CYAN}========================================{RESET}\n");
    println!("{YELLOW}{title}{RESET}\n");
}

fn help() {
    println!("\n{CYAN}========================================{RESET}");
    println!("{WHITE}             CustomAIO{RESET}");
    println!("{CYAN}========================================{RESET}\n");
    println!("  {GREEN}fan silent{RESET}       Apply the Silent profile");
    println!("  {ORANGE}fan perf{RESET}         Apply the Performance profile");
    println!("  {CYAN}fan status{RESET}       Cooler, profile and temperatures");
    println!("  {WHITE}fan lcd{RESET}          Run the display service");
    println!("  {WHITE}fan preview{RESET}      Render a frame to PNG, no device needed");
    println!("  {WHITE}fan devices{RESET}      List detected coolers");
    println!("  {GRAY}fan help{RESET}         Show this menu\n");
    println!("  {GRAY}Settings live in config.toml next to this program.{RESET}");
    println!("  {GRAY}`fan preview dial` previews one style without changing it.{RESET}\n");
}
