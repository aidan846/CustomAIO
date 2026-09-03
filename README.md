# CustomAIO (Rust)

Fan/pump profiles and an LCD temperature readout for NZXT Kraken coolers, as a
single ~1 MB executable with no runtime to install.

This is a rewrite of the Python CustomAIO. It drops Python, liquidctl,
LibreHardwareMonitor and the .NET runtime entirely, and talks to the hardware
directly.

## Commands

Once the folder is on your PATH:

```
customaio silent      Apply the Silent profile
customaio perf        Apply the Performance profile
customaio status      Cooler, service state, profile and temperatures
customaio start       Start the display service in the background
customaio stop        Stop the display service
customaio restart     Restart it, picking up config.toml changes
customaio lcd         Run it in this window, printing each reading
customaio preview     Render a frame to PNG without a device
customaio devices     List detected coolers
customaio package     Build a distributable zip
customaio help
```

`start` runs the service detached, so it keeps going after you close the
terminal. `lcd` is the opposite: it runs in the foreground and prints every
reading, which is what you want when testing a style or diagnosing a sensor -
close the window and it stops. Only one of them can run at a time; whichever
starts second reports the conflict rather than fighting for the USB device.

`start` prefers the logon task, because that runs elevated and CPU
temperature needs elevation. If no task is installed it still launches
detached, and says that CPU temperature will read `N/A` unless you started it
from an administrator prompt.

`setup.bat` also writes a one-line `fan.bat` beside the executable, so
`fan silent`, `fan perf` and the rest work as a shorter alias for all of these.

`customaio preview dial` renders one style without changing your config.

## Install

1. `cargo build --release`
2. Right-click `setup.bat`, **Run as administrator**, choose **Full setup**.
   That copies the binary up from `target\release`, writes the `fan.bat`
   alias, adds the folder to PATH, and registers a logon task that runs
   `customaio lcd --quiet` elevated.
3. Open a new terminal and run `customaio status`.

Close NZXT CAM and the old Python service first - only one program can drive
the cooler at a time.

## Building a release

```
cargo dist
```

or double-click `build.bat`, which does the same thing.

Either one builds in release mode and then assembles everything a user needs
into `target\release\CustomAIO\`, zipping it to `target\release\CustomAIO.zip`
(about 0.5 MB), ready to attach to a GitHub release.

**`cargo build --release` on its own is not enough** - it only produces
`target\release\customaio.exe`, with no package folder and no zip.

The package holds `customaio.exe`, `fan.bat`, `config.toml` (pristine defaults, not your local
copy), `setup.bat`, `modules\`, README and LICENSE. It is self-contained:
copy the folder anywhere, or unzip the archive, and run `setup.bat` as
administrator.

`cargo dist` is an alias defined in `.cargo\config.toml`, and `build.bat` runs
the same two steps. Cargo has no post-build hook and will not let an alias
shadow `build`, so packaging genuinely cannot be attached to `cargo build`
itself - it has to be a separate command.

Note that `cargo clean` deletes everything under `target`, this package
included, so move a release you want to keep out of there first.

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

Set `[fan] enabled = false`. `customaio silent` and `customaio perf` then
refuse to run and nothing touches the speed channels; the LCD service is
unaffected. Likewise `[display] lcd = false` runs it as a sensor-to-PNG
service with no cooler.

## Supported hardware

Fully implemented and tested on the **Kraken Z53/Z63/Z73** (`1E71:3008`).

The Kraken 2023/2024 models are recognised, and fan/pump control uses their
channel IDs, but their firmware-2 image upload path is not implemented -
`customaio lcd` reports that clearly rather than failing silently. Everything
outside the Kraken family is unsupported.

## Layout

```
customaio.exe    the whole program
fan.bat          one-line `fan` alias, written by setup.bat
config.toml      everything you can tune
setup.bat        PATH, fan.bat, logon task, PawnIO check
build.bat        build + package in one step (same as `cargo dist`)
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
