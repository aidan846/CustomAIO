//! Config file loading. Everything the user can tune lives in config.toml,
//! next to the executable. Missing keys fall back to the defaults below, so a
//! partial (or absent) config file is always valid.

use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const DEFAULT_CONFIG: &str = include_str!("../config.toml");

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub general: General,
    pub display: Display,
    pub device: Device,
    pub sensors: Sensors,
    pub style: Style,
    pub fan: Fan,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct General {
    /// Seconds between frames. The service sleeps the rest of the time.
    pub update_interval: f64,
    /// Write data/customaio.log. Off means zero disk writes while idle.
    pub log: bool,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Display {
    /// Push frames to the cooler's LCD.
    pub lcd: bool,
    /// Also write each frame to disk, for previewing styles or feeding OBS.
    pub save_png: bool,
    pub png_path: String,
    /// Degrees, one of 0/90/180/270. Applied while rendering, so the device
    /// itself is always left at orientation 0.
    pub rotation: u32,
    /// LCD backlight, 0-100.
    pub brightness: u8,
}

/// Which cooler to talk to. All blank/zero means "use the first one found".
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Device {
    pub vendor_id: u16,
    pub product_id: u16,
    pub serial: String,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Sensors {
    /// auto | pawnio | none
    pub cpu: String,
    /// auto | nvidia | none
    pub gpu: String,
    /// Folder holding the PawnIO .bin modules.
    pub pawnio_modules: String,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Style {
    /// classic | stacked | dial | minimal
    pub name: String,
    pub colors: Colors,
    pub options: Options,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Colors {
    pub background: String,
    pub text: String,
    pub accent: String,
    pub accent_gpu: String,
    pub track: String,
    pub alert_background: String,
    pub alert_text: String,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub show_cpu: bool,
    pub show_gpu: bool,
    pub show_bars: bool,
    pub show_labels: bool,
    pub show_scale: bool,
    pub show_degree_symbol: bool,
    /// Turn the whole screen into a warning above the alert temperatures.
    pub alert_enabled: bool,
    pub cpu_alert_temp: f32,
    pub gpu_alert_temp: f32,
    /// Range the bars/dials map onto.
    pub bar_min: f32,
    pub bar_max: f32,
    /// Tint the readout toward the accent colour as it heats up.
    pub color_by_temp: bool,
    pub font: String,
    pub font_bold: String,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Fan {
    /// When false, `fan silent`/`fan perf` refuse to run and the LCD service
    /// never touches the speed channels.
    pub enabled: bool,
    pub silent: Profile,
    pub performance: Profile,
}

/// A profile is a duty curve per channel: a list of [liquid_temp, duty%] points,
/// interpolated by the firmware between 20 and 59 C.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    pub pump: Vec<[u8; 2]>,
    pub fan: Vec<[u8; 2]>,
}

impl Default for General {
    fn default() -> Self {
        Self { update_interval: 1.0, log: true }
    }
}

impl Default for Display {
    fn default() -> Self {
        Self {
            lcd: true,
            save_png: true,
            png_path: "data/frame.png".into(),
            rotation: 90,
            brightness: 100,
        }
    }
}

impl Default for Device {
    fn default() -> Self {
        Self { vendor_id: 0, product_id: 0, serial: String::new() }
    }
}

impl Default for Sensors {
    fn default() -> Self {
        Self { cpu: "auto".into(), gpu: "auto".into(), pawnio_modules: "modules".into() }
    }
}

impl Default for Style {
    fn default() -> Self {
        Self { name: "classic".into(), colors: Colors::default(), options: Options::default() }
    }
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            background: "#000000".into(),
            text: "#FFFFFF".into(),
            accent: "#FF7A1A".into(),
            accent_gpu: "#FF7A1A".into(),
            track: "#202020".into(),
            alert_background: "#FF0000".into(),
            alert_text: "#FFFFFF".into(),
        }
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            show_cpu: true,
            show_gpu: true,
            show_bars: true,
            show_labels: true,
            show_scale: true,
            show_degree_symbol: true,
            alert_enabled: true,
            cpu_alert_temp: 85.0,
            gpu_alert_temp: 85.0,
            bar_min: 20.0,
            bar_max: 90.0,
            color_by_temp: false,
            font: r"C:\Windows\Fonts\segoeui.ttf".into(),
            font_bold: r"C:\Windows\Fonts\segoeuib.ttf".into(),
        }
    }
}

impl Default for Fan {
    fn default() -> Self {
        Self {
            enabled: true,
            silent: Profile {
                pump: vec![[20, 60], [35, 60], [40, 70], [45, 80], [50, 90], [55, 100]],
                fan: vec![[20, 30], [35, 30], [40, 40], [45, 55], [50, 75], [55, 100]],
            },
            performance: Profile {
                pump: vec![[20, 70], [35, 70], [40, 80], [45, 85], [50, 90], [55, 95], [60, 100]],
                fan: vec![[20, 50], [35, 50], [40, 60], [45, 70], [50, 80], [55, 90], [60, 100]],
            },
        }
    }
}

impl Default for Profile {
    fn default() -> Self {
        Self { pump: Vec::new(), fan: Vec::new() }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: General::default(),
            display: Display::default(),
            device: Device::default(),
            sensors: Sensors::default(),
            style: Style::default(),
            fan: Fan::default(),
        }
    }
}

/// The directory everything is read from and written to: the one holding
/// config.toml. In a normal install that is the executable's own folder; in a
/// source checkout the executable lives in target/release, so the working
/// directory wins instead. Resolved once, so every path agrees.
pub fn base_dir() -> &'static Path {
    static BASE: OnceLock<PathBuf> = OnceLock::new();
    BASE.get_or_init(|| {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf));
        if let Some(dir) = &exe_dir {
            if dir.join("config.toml").exists() {
                return dir.clone();
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            if cwd.join("config.toml").exists() {
                return cwd;
            }
        }
        // No config anywhere yet; the executable's folder is where one will
        // be written on first run.
        exe_dir.unwrap_or_else(|| PathBuf::from("."))
    })
}

/// Resolve a config path against `base_dir`, leaving absolute paths alone.
pub fn resolve(rel: &str) -> PathBuf {
    let p = Path::new(rel);
    if p.is_absolute() { p.to_path_buf() } else { base_dir().join(p) }
}

impl Config {
    /// Load config.toml, writing a commented default file if none exists.
    pub fn load() -> Result<Self, String> {
        let path = resolve("config.toml");
        if !path.exists() {
            // Seed a fully commented file so the options are discoverable.
            let _ = std::fs::write(&path, DEFAULT_CONFIG);
            return Ok(Config::default());
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("could not read {}: {e}", path.display()))?;
        toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// Parse "#RRGGBB" (or "#RGB") into a tiny-skia colour, falling back to magenta
/// so a typo in the config is visible rather than silent.
pub fn color(hex: &str) -> tiny_skia::Color {
    let h = hex.trim().trim_start_matches('#');
    let expand = |c: u8| c * 17;
    let (r, g, b) = match h.len() {
        3 => {
            let v = u16::from_str_radix(h, 16).unwrap_or(0xF0F);
            (expand((v >> 8) as u8 & 0xF), expand((v >> 4) as u8 & 0xF), expand(v as u8 & 0xF))
        }
        6 => match u32::from_str_radix(h, 16) {
            Ok(v) => ((v >> 16) as u8, (v >> 8) as u8, v as u8),
            Err(_) => (255, 0, 255),
        },
        _ => (255, 0, 255),
    };
    tiny_skia::Color::from_rgba8(r, g, b, 255)
}
