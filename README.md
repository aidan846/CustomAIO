# CustomAIO (Rust)

Fan/pump profiles and an LCD temperature readout for NZXT Kraken coolers, as a
single ~1 MB executable with no runtime to install.

This is a rewrite of the Python CustomAIO. It drops Python, liquidctl,
LibreHardwareMonitor and the .NET runtime entirely, and talks to the hardware
directly.

## Commands

The binary is named `fan`, so once the folder is on your PATH:

```
fan silent      Apply the Silent profile
fan perf        Apply the Performance profile
fan status      Cooler, active profile and temperatures
fan lcd         Run the display service
fan preview     Render a frame to PNG without a device
fan devices     List detected coolers
fan help
```

`fan preview dial` renders one style without changing your config.

## Install

1. `cargo build --release`
2. Right-click `setup.bat`, **Run as administrator**, choose **Full setup**.
   That copies the binary up from `target\release`, adds the folder to PATH,
   and registers a logon task that runs `fan lcd --quiet` elevated.
3. Open a new terminal and run `fan status`.

Close NZXT CAM and the old Python service first - only one program can drive
the cooler at a time.

## Building a release

```
cargo build --release
target\release\fan.exe package
```

That assembles `dist\CustomAIO\` - the executable, `config.toml` with
pristine defaults, `setup.bat`, the PawnIO modules, README and LICENSE - and
zips it to `dist\CustomAIO.zip`, ready to attach to a GitHub release. The
folder is self-contained: unzip anywhere and run `setup.bat`.

It writes to `dist\` rather than `target\` because `cargo clean` wipes
`target`, which would delete a release you were about to upload.

## Where temperatures come from

**CPU** - the [PawnIO](https://pawnio.eu) kernel driver reads the vendor's
thermal registers directly: `IA32_PACKAGE_THERM_STATUS` against `TjMax` on
Intel, the `THM_TCON_CUR_TMP` SMN register on AMD. The matching PawnIO module
is picked from `modules/` based on CPUID, so no configuration is needed. This
is the same mechanism LibreHardwareMonitor uses, without the .NET runtime.
PawnIO only talks to elevated callers, which is why the logon task runs with
highest privileges. Without it, CPU shows `N/A`.

**GPU** - NVML (`nvml.dll`) on NVIDIA. On any other adapter it falls back to
D3DKMT, the WDDM interface Task Manager reads, which reports a temperature for
AMD and Intel GPUs without a vendor SDK. Set `gpu = "wddm"` to force the
neutral path. The old version shelled out to `nvidia-smi` once per tick; this
one does neither a process spawn nor an allocation.

## Styles

Set `[style] name` in `config.toml`:

| Style | Layout |
| --- | --- |
| `classic` | The original: label, big number, horizontal bar. Default. |
| `stacked` | Large numbers over their labels, no bars. |
| `dial` | Circular gauges. |
| `minimal` | Just the two numbers. |

Every style is laid out against a 320x320 grid and scaled to the panel, so the
same style works on a 240x240 or 640x640 screen. Colours, the alert
thresholds, the bar range, and which elements are drawn at all are under
`[style.colors]` and `[style.options]`.

## Cost

Measured on this machine at a 1 second update interval:

| | Working set | CPU |
| --- | --- | --- |
| Python version (10s interval) | 93 MB | - |
| This, `save_png = true` | 30 MB | 1.1% of one core |
| This, `save_png = false` | 30 MB | 0.16% of one core |

PNG encoding dominates the CPU cost, so turn `save_png` off once your style is
settled. The frame buffer, fonts and pixel staging buffer are all allocated
once and reused, so a steady-state frame allocates nothing.

## Skipping fan control

Set `[fan] enabled = false`. `fan silent` and `fan perf` then refuse to run and
nothing touches the speed channels; the LCD service is unaffected. Likewise
`[display] lcd = false` runs it as a sensor-to-PNG service with no cooler.

## Supported hardware

Fully implemented and tested on the **Kraken Z53/Z63/Z73** (`1E71:3008`).

The Kraken 2023/2024 models are recognised, and fan/pump control uses their
channel IDs, but their firmware-2 image upload path is not implemented - `fan
lcd` reports that clearly rather than failing silently. Everything outside the
Kraken family is unsupported.

## Layout

```
fan.exe          the whole program
config.toml      everything you can tune
setup.bat        PATH, logon task, PawnIO check
modules/         PawnIO blobs for CPU temperature
data/            frame.png, customaio.log, profile.txt
src/
  main.rs        CLI and the service loop
  config.rs      config parsing and defaults
  sensors.rs     PawnIO (CPU) and NVML/D3DKMT (GPU)
  render.rs      the styles
  kraken.rs      HID commands, fan curves, LCD upload
```

The wire protocol follows liquidctl's `kraken3` driver.
