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
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Print a line, dropping colour codes when the terminal cannot render them.
/// Every user-facing message goes through this rather than `println!`.
macro_rules! say {
    () => { emit(String::new()) };
    ($($arg:tt)*) => { emit(format!($($arg)*)) };
}

/// Whether ANSI escapes will actually be rendered.
///
/// Windows consoles ignore escape sequences and print them literally unless
/// virtual terminal processing is enabled, so we turn it on and report whether
/// that worked. `GetConsoleMode` also fails when stdout is a file or a pipe,
/// which is exactly when colour codes would be noise, so the same check covers
/// redirection. `NO_COLOR` is honoured by convention.
fn colors_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        if std::env::var_os("NO_COLOR").is_some() {
            return false;
        }
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
            use windows_sys::Win32::System::Console::{
                GetConsoleMode, GetStdHandle, SetConsoleMode,
                ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_OUTPUT_HANDLE,
            };
            let handle = GetStdHandle(STD_OUTPUT_HANDLE);
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                return false;
            }
            let mut mode = 0u32;
            if GetConsoleMode(handle, &mut mode) == 0 {
                return false;
            }
            if mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0 {
                return true;
            }
            SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
        }
        #[cfg(not(windows))]
        true
    })
}

/// Remove `ESC [ ... m` sequences, for terminals that would print them raw.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        // Skip "[ <digits and semicolons> m".
        if chars.next() != Some('[') {
            continue;
        }
        for c in chars.by_ref() {
            if c.is_ascii_alphabetic() {
                break;
            }
        }
    }
    out
}

fn emit(line: String) {
    if colors_enabled() {
        println!("{line}");
    } else {
        println!("{}", strip_ansi(&line));
    }
}

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
        "package" => package(),
        "help" | "-h" | "--help" => {
            help();
            Ok(())
        }
        other => Err(format!("unknown command '{other}'. Try `fan help`.")),
    };

    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            let msg = format!("{RED}Error:{RESET} {e}");
            eprintln!("{}", if colors_enabled() { msg } else { strip_ansi(&msg) });
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
    say!("  {GRAY}{}{RESET}", device.model.name);

    // Pump first: it should already be ramping before the fans follow.
    device.set_curve(kraken::Channel::Pump, &profile.pump)?;
    say!("  pump curve set");
    device.set_curve(kraken::Channel::Fan, &profile.fan)?;
    say!("  fan curve set");

    let _ = std::fs::write(data_path("profile.txt"), name);
    say!("\n{GREEN}{name} profile applied.{RESET}\n");
    Ok(())
}

fn status() -> Result<(), String> {
    let cfg = config::Config::load()?;
    header("Status");

    let last = std::fs::read_to_string(data_path("profile.txt")).unwrap_or_else(|_| "unknown".into());
    let device = kraken::Kraken::open(&cfg.device)?;
    say!("  {WHITE}Device{RESET}        {}", device.model.name);
    say!("  {WHITE}Serial{RESET}        {}", device.serial);
    say!("  {WHITE}Last profile{RESET}  {}", last.trim());

    match device.status() {
        Ok(s) => {
            say!("  {WHITE}Liquid{RESET}        {:.1} C", s.liquid_temp);
            say!("  {WHITE}Pump{RESET}          {} RPM ({}%)", s.pump_rpm, s.pump_duty);
            say!("  {WHITE}Fan{RESET}           {} RPM ({}%)", s.fan_rpm, s.fan_duty);
        }
        Err(e) => say!("  {GRAY}cooler status unavailable - {e}{RESET}"),
    }

    let (readings, notes) = sensors::Readings::open(&cfg.sensors);
    let (cpu, gpu) = readings.sample();
    say!("  {WHITE}CPU{RESET}           {}", temp_or_na(cpu));
    say!("  {WHITE}GPU{RESET}           {}", temp_or_na(gpu));
    for n in notes {
        say!("  {GRAY}{n}{RESET}");
    }
    say!();
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
        say!("  {WHITE}{}{RESET}", model.name);
        say!(
            "    VID 0x{:04X}  PID 0x{:04X}  serial {}",
            model.vid,
            model.pid,
            if serial.is_empty() { "not reported" } else { &serial }
        );
        say!(
            "    LCD {}x{}{}",
            model.resolution.0,
            model.resolution.1,
            if model.modern { "  (image upload not yet implemented)" } else { "" }
        );
    }
    if found == 0 {
        say!("  {GRAY}none found - check the USB header and close NZXT CAM{RESET}");
    }
    say!();
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
    say!("Style '{}' rendered to {}", cfg.style.name, out.display());
    Ok(())
}

/// Assemble everything a user needs to run CustomAIO into
/// target\release\CustomAIO, then zip it alongside. The folder is
/// self-contained: copy it anywhere, or unzip the archive, and run setup.bat.
///
/// Note that `cargo clean` deletes everything under target, this package
/// included, so move a release you intend to keep out of there.
fn package() -> Result<(), String> {
    let root = config::base_dir();
    let dest = root.join("target").join("release");
    let out = dest.join("CustomAIO");
    header("Building release package");

    // Start clean, so a file removed from the project can't linger in a
    // release from a previous run.
    if out.exists() {
        std::fs::remove_dir_all(&out).map_err(|e| format!("could not clear {}: {e}", out.display()))?;
    }
    std::fs::create_dir_all(out.join("modules"))
        .map_err(|e| format!("could not create {}: {e}", out.display()))?;

    // The running executable is the one that was just built, so copying it
    // avoids guessing at debug vs release paths.
    let exe = std::env::current_exe().map_err(|e| format!("could not locate fan.exe: {e}"))?;
    copy_into(&exe, &out.join("fan.exe"))?;
    say!("  fan.exe");

    for name in ["setup.bat", "README.md", "LICENSE"] {
        let src = root.join(name);
        if src.exists() {
            copy_into(&src, &out.join(name))?;
            say!("  {name}");
        } else {
            say!("  {GRAY}{name} missing, skipped{RESET}");
        }
    }

    // Ship pristine defaults rather than the local config, which holds this
    // machine's device serial and personal tweaks.
    std::fs::write(out.join("config.toml"), config::DEFAULT_CONFIG)
        .map_err(|e| format!("could not write config.toml: {e}"))?;
    say!("  config.toml {GRAY}(defaults, not your local copy){RESET}");

    let modules = root.join("modules");
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(&modules) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "bin") {
                copy_into(&path, &out.join("modules").join(entry.file_name()))?;
                count += 1;
            }
        }
    }
    say!("  modules/ {GRAY}({count} PawnIO modules){RESET}");
    if count == 0 {
        say!("  {GRAY}warning: without these, CPU temperature reads N/A{RESET}");
    }

    // Compress-Archive ships with Windows, so this needs nothing installed.
    let zip = dest.join("CustomAIO.zip");
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(format!(
            "Compress-Archive -Path '{}\\*' -DestinationPath '{}' -Force",
            out.display(),
            zip.display()
        ))
        .status();

    say!("\n{GREEN}Package ready.{RESET}");
    say!("  {WHITE}Folder{RESET}  {}", out.display());
    match status {
        Ok(s) if s.success() && zip.exists() => {
            let size = std::fs::metadata(&zip).map(|m| m.len()).unwrap_or(0);
            say!("  {WHITE}Zip{RESET}     {} ({:.1} MB)", zip.display(), size as f64 / 1_048_576.0);
            say!("\n  {GRAY}Attach the zip to a GitHub release.{RESET}");
        }
        _ => {
            say!("  {GRAY}Could not create the zip; compress the folder yourself.{RESET}");
        }
    }
    say!("  {GRAY}To use: unzip anywhere, then run setup.bat as administrator.{RESET}\n");
    Ok(())
}

fn copy_into(src: &std::path::Path, dst: &PathBuf) -> Result<(), String> {
    std::fs::copy(src, dst)
        .map(|_| ())
        .map_err(|e| format!("could not copy {} -> {}: {e}", src.display(), dst.display()))
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
            emit(line.to_string());
        }
        if let Some(f) = self.file.as_mut() {
            // The log file never wants escape codes, whatever the console does.
            let _ = writeln!(f, "{}", strip_ansi(line));
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
    say!("\n{CYAN}========================================{RESET}");
    say!("{WHITE}             CustomAIO{RESET}");
    say!("{CYAN}========================================{RESET}\n");
    say!("{YELLOW}{title}{RESET}\n");
}

fn help() {
    say!("\n{CYAN}========================================{RESET}");
    say!("{WHITE}             CustomAIO{RESET}");
    say!("{CYAN}========================================{RESET}\n");
    say!("  {GREEN}fan silent{RESET}       Apply the Silent profile");
    say!("  {ORANGE}fan perf{RESET}         Apply the Performance profile");
    say!("  {CYAN}fan status{RESET}       Cooler, profile and temperatures");
    say!("  {WHITE}fan lcd{RESET}          Run the display service");
    say!("  {WHITE}fan preview{RESET}      Render a frame to PNG, no device needed");
    say!("  {WHITE}fan devices{RESET}      List detected coolers");
    say!("  {WHITE}fan package{RESET}      Build a zip you can share");
    say!("  {GRAY}fan help{RESET}         Show this menu\n");
    say!("  {GRAY}Settings live in config.toml next to this program.{RESET}");
    say!("  {GRAY}`fan preview dial` previews one style without changing it.{RESET}\n");
}
