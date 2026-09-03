//! CustomAIO - fan/pump profiles and an LCD readout for NZXT Kraken coolers.
//!
//! One binary. setup.bat adds a fan.bat shim beside it, so both the full
//! name and the shorter `fan` spelling work:
//!
//!     customaio silent    customaio status    customaio lcd
//!     fan silent          fan status          fan lcd
//!
//! `customaio lcd` is the long-running service; everything else is a one-shot.

// Keep the console window from flashing when Task Scheduler starts the
// service, while still behaving like a console app when launched from a
// terminal. `customaio lcd --quiet` is what the scheduled task uses.
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
        "start" => start_service(),
        "stop" => stop_service(),
        "restart" => restart_service(),
        "lcd" | "service" => service(rest),
        "preview" => preview(rest),
        "devices" | "list" => devices(),
        "package" => package(),
        "help" | "-h" | "--help" => {
            help();
            Ok(())
        }
        other => Err(format!("unknown command '{other}'. Try `customaio help`.")),
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
    if service_running() {
        say!("  {WHITE}Service{RESET}       {GREEN}running{RESET}");
    } else {
        say!("  {WHITE}Service{RESET}       {GRAY}stopped{RESET}  (start it with `customaio start`)");
    }
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
/// styles and colours. `customaio preview dial` overrides the configured style.
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
    let exe = std::env::current_exe().map_err(|e| format!("could not locate customaio.exe: {e}"))?;
    copy_into(&exe, &out.join("customaio.exe"))?;
    say!("  customaio.exe");

    // The `fan` alias, so the package works before setup.bat has run.
    std::fs::write(
        out.join("fan.bat"),
        "@echo off\r\n\
         rem Shorter alias for customaio.exe, beside this file.\r\n\
         \"%~dp0customaio.exe\" %*\r\n",
    )
    .map_err(|e| format!("could not write fan.bat: {e}"))?;
    say!("  fan.bat {GRAY}(so `fan silent` works too){RESET}");

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
// Starting and stopping the background service
// ============================================================

const TASK_NAME: &str = "CustomAIO LCD";

/// Run a helper process without flashing a console window.
fn quiet_command(program: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn logon_task_exists() -> bool {
    quiet_command("schtasks")
        .args(["/Query", "/TN", TASK_NAME])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Start the display service in the background.
///
/// The logon task is preferred because it runs elevated, which is what makes
/// CPU temperature readable. Without it we still detach a process, but say so,
/// since it inherits this terminal's privileges.
fn start_service() -> Result<(), String> {
    header("Starting the display service");

    if service_running() {
        say!("  {GRAY}Already running. Use `customaio restart` to reload config.toml.{RESET}\n");
        return Ok(());
    }

    if logon_task_exists() {
        let ok = quiet_command("schtasks")
            .args(["/Run", "/TN", TASK_NAME])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if ok {
            say!("  {GREEN}Started via the \"{TASK_NAME}\" task.{RESET}");
            say!("  {GRAY}It runs elevated, so CPU temperature is available.{RESET}\n");
            return Ok(());
        }
        say!("  {GRAY}The scheduled task would not start; launching directly.{RESET}");
    }

    // Detach fully, so closing this terminal does not take the service with it.
    let exe = std::env::current_exe().map_err(|e| format!("could not locate customaio.exe: {e}"))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["lcd", "--quiet"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    cmd.spawn().map_err(|e| format!("could not start the service: {e}"))?;

    say!("  {GREEN}Started in the background.{RESET}");
    if !logon_task_exists() {
        say!("  {GRAY}No logon task exists, so this will not come back after a reboot.{RESET}");
        say!("  {GRAY}Run setup.bat as administrator to install one.{RESET}");
    }
    say!("  {GRAY}Started from this terminal, so it inherits your privileges;{RESET}");
    say!("  {GRAY}CPU temperature needs an elevated prompt or the logon task.{RESET}\n");
    Ok(())
}

/// Stop the service, whether it came from the logon task or a direct launch.
fn stop_service() -> Result<(), String> {
    header("Stopping the display service");

    if logon_task_exists() {
        let _ = quiet_command("schtasks").args(["/End", "/TN", TASK_NAME]).output();
        say!("  {GRAY}Ended the \"{TASK_NAME}\" task.{RESET}");
    }

    // Kill any remaining instance, skipping this process. fan.exe is the
    // pre-rename name and may still be installed.
    let me = std::process::id();
    let mut killed = false;
    for image in ["customaio.exe", "fan.exe"] {
        let out = quiet_command("taskkill")
            .args(["/F", "/IM", image, "/FI", &format!("PID ne {me}")])
            .output();
        if let Ok(o) = out {
            if o.status.success() {
                killed = true;
            }
        }
    }
    if killed {
        say!("  {GRAY}Stopped a running instance.{RESET}");
    }

    // The lock is released as the process exits, which lags taskkill by a
    // moment; poll briefly rather than reporting a failure that isn't one.
    for _ in 0..30 {
        if !service_running() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    if service_running() {
        say!("\n  {GRAY}Something still holds the service lock. If it was started{RESET}");
        say!("  {GRAY}elevated, stop it from an administrator prompt.{RESET}\n");
    } else {
        say!("\n{GREEN}Service stopped.{RESET}\n");
    }
    Ok(())
}

fn restart_service() -> Result<(), String> {
    stop_service()?;
    // Give the old process a moment to release the cooler's USB interfaces.
    std::thread::sleep(Duration::from_millis(1500));
    start_service()
}

// ============================================================
// The LCD service
// ============================================================

/// Refuse to start a second display service.
///
/// Two instances fight over the cooler's HID and bulk interfaces, and the
/// loser reports missing replies and an unopenable bulk endpoint - which reads
/// like a driver problem rather than the contention it actually is. A named
/// mutex in the session namespace catches it up front.
///
/// The scheduled task runs elevated while a hand-run copy usually is not, so
/// the mutex may already exist at a higher integrity level and come back as
/// access-denied instead of already-exists. Both mean the same thing here.
#[cfg(windows)]
const SERVICE_MUTEX: &str = "Local\\CustomAIO.LcdService\0";

/// Take the service lock, reporting whether someone already held it. The
/// handle is deliberately leaked so the lock lives as long as the process.
#[cfg(windows)]
fn claim_service_lock() -> bool {
    use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, GetLastError};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let name: Vec<u16> = SERVICE_MUTEX.encode_utf16().collect();
    unsafe {
        let handle = CreateMutexW(std::ptr::null(), 1, name.as_ptr());
        let err = GetLastError();
        if handle.is_null() {
            return err == ERROR_ACCESS_DENIED;
        }
        err == ERROR_ALREADY_EXISTS
    }
}

/// Ask whether a service is running *without* taking the lock. `start` must
/// use this: creating the mutex here would leave this process holding it, and
/// the service it then spawns would see the lock and refuse to run.
#[cfg(windows)]
fn service_running() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, GetLastError};
    use windows_sys::Win32::System::Threading::OpenMutexW;

    // windows-sys only exposes SYNCHRONIZE under an unrelated file-system
    // feature, so use the value directly rather than pulling that in.
    const SYNCHRONIZE: u32 = 0x0010_0000;

    let name: Vec<u16> = SERVICE_MUTEX.encode_utf16().collect();
    unsafe {
        let handle = OpenMutexW(SYNCHRONIZE, 0, name.as_ptr());
        if handle.is_null() {
            // The elevated task's mutex can be unopenable from a normal
            // prompt; that still means a service is running.
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        CloseHandle(handle);
        true
    }
}

#[cfg(not(windows))]
fn claim_service_lock() -> bool {
    false
}

#[cfg(not(windows))]
fn service_running() -> bool {
    false
}

fn service(args: &[String]) -> Result<(), String> {
    let quiet = args.iter().any(|a| a == "--quiet" || a == "-q");
    if claim_service_lock() {
        return Err(
            "the CustomAIO display service is already running, most likely the \
             \"CustomAIO LCD\" scheduled task.\n       Two copies fight over the cooler's USB \
             interfaces, so this one stopped instead.\n       Stop it first with:  customaio stop\
             \n       Or use `customaio status`, which does not conflict."
                .into(),
        );
    }
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
    say!("  {GREEN}customaio silent{RESET}    Apply the Silent profile");
    say!("  {ORANGE}customaio perf{RESET}      Apply the Performance profile");
    say!("  {CYAN}customaio status{RESET}    Cooler, profile and temperatures");
    say!("  {GREEN}customaio start{RESET}     Start the display service in the background");
    say!("  {ORANGE}customaio stop{RESET}      Stop the display service");
    say!("  {WHITE}customaio restart{RESET}   Restart it, picking up config.toml changes");
    say!("  {WHITE}customaio lcd{RESET}       Run it in this window, printing readings");
    say!("  {WHITE}customaio preview{RESET}   Render a frame to PNG, no device needed");
    say!("  {WHITE}customaio devices{RESET}   List detected coolers");
    say!("  {WHITE}customaio package{RESET}   Build a zip you can share");
    say!("  {GRAY}customaio help{RESET}      Show this menu\n");
    say!("  {GRAY}`fan` is a shorter alias for all of the above:{RESET}");
    say!("  {GRAY}fan silent, fan perf, fan status ...{RESET}\n");
    say!("  {GRAY}Settings live in config.toml next to this program.{RESET}");
    say!("  {GRAY}`fan preview dial` previews one style without changing it.{RESET}\n");
}
