# CustomAIO

Fan and pump profiles, plus a temperature readout on the cooler's LCD, for
NZXT Kraken coolers on Windows.

One ~1 MB executable. No Python, no .NET, nothing to install alongside it.
Updating once a second, it sits at about 30 MB and roughly 1% of one CPU core,
or a sixth of that with `save_png` turned off.

## Install

1. Download the zip from Releases, or build it yourself (see below), and
   unpack it wherever you want it to live.
2. Right-click `setup.bat` and choose **Run as administrator**, then pick
   **Full setup**.
3. Open a new terminal and run `customaio status`.

Setup adds the folder to your PATH and registers a task that starts the
display service when you log in. Keep the folder where it is afterwards - the
task remembers its full path.

Close NZXT CAM first. Only one program can drive the cooler at a time.

## Commands

| Command | What it does |
| --- | --- |
| `customaio silent` | Apply the Silent fan/pump curves |
| `customaio perf` | Apply the Performance curves |
| `customaio status` | Service state, cooler readings, CPU and GPU |
| `customaio start` | Start the display service in the background |
| `customaio stop` | Stop it |
| `customaio restart` | Restart it, picking up `config.toml` changes |
| `customaio lcd` | Run it in this window, printing each reading |
| `customaio preview` | Render a frame to `data\frame.png`, no cooler needed |
| `customaio devices` | List detected coolers |
| `customaio help` | The above, in the terminal |

`fan` is a shorter alias for all of them: `fan silent`, `fan status`, and so on.

`start` runs detached, so it keeps going after you close the terminal. `lcd`
is the opposite - it runs in the foreground and prints every reading, which is
what you want when testing a style. Only one can run at a time; whichever
starts second says so instead of fighting over the USB device.

## CPU temperature needs administrator rights

CPU temperature is read from the CPU's own thermal registers through the
[PawnIO](https://pawnio.eu) driver, and that driver only talks to elevated
programs. There is no way around it.

- **The logon task already runs elevated**, so the LCD shows CPU temperature
  normally. Nothing to do.
- **Typing `customaio status` in an ordinary terminal shows `CPU N/A`.** That
  is expected. Open Command Prompt or PowerShell as administrator and run it
  again to see the reading.

If PawnIO is not installed, `setup.bat` option **5** tells you so. Get it from
[pawnio.eu](https://pawnio.eu) and run setup again. Without it, CPU shows
`N/A` everywhere; everything else still works.

GPU temperature needs no special rights. It uses NVIDIA's NVML where
available, and otherwise the same Windows interface Task Manager uses, which
covers AMD and Intel cards.

## Settings

Everything lives in `config.toml`, next to the executable. Edit it, then run
`customaio restart`.

The things people usually change:

```toml
[display]
rotation = 90        # 0, 90, 180 or 270 - turn until it reads upright
save_png = true      # also write data\frame.png
brightness = 100

[style]
name = "classic"     # classic | stacked | dial | minimal

[style.colors]
background = "#000000"
text = "#FFFFFF"
accent = "#FF7A1A"
```

| Style | Layout |
| --- | --- |
| `classic` | Label, big number, horizontal bar. The default. |
| `stacked` | Large numbers over their labels, no bars. |
| `dial` | Circular gauges. |
| `minimal` | Just the two numbers. |

`customaio preview dial` renders a style to `data\frame.png` so you can look
at it without changing your config.

`[style.options]` turns individual pieces on and off - labels, bars, the
`20C`/`90C` scale, the overheat warning screen and its thresholds.

Two more worth knowing:

- **`save_png = false`** makes it about seven times cheaper on CPU. PNG
  encoding costs more than everything else put together, so turn it off once
  your style is settled.
- **`[fan] enabled = false`** switches fan control off entirely. `silent` and
  `perf` then refuse to run and nothing touches the speed channels. The
  display is unaffected. Likewise `[display] lcd = false` runs it as a
  sensors-to-PNG service with no cooler involved.

Fan curves are `[liquid temperature, duty %]` points under `[fan.silent]` and
`[fan.performance]`. The cooler interpolates between them.

## Supported hardware

Written for and tested on the **Kraken Z53, Z63 and Z73**.

The Kraken 2023 and 2024 models are recognised and their fan and pump control
works, but their LCD uses a different image protocol that is not implemented -
the service says so plainly rather than failing quietly. Other brands are not
supported.

## Building from source

You need [Rust](https://rustup.rs). Then either double-click `build.bat` or
run:

```
cargo dist
```

Both compile in release mode and assemble `target\release\CustomAIO\` plus
`CustomAIO.zip`, ready to attach to a GitHub release.

Plain `cargo build --release` only produces `customaio.exe` - Cargo cannot run
a step after a build, so packaging has to be its own command.

## Files

```
customaio.exe    the whole program
config.toml      everything you can change
setup.bat        PATH, logon task, PawnIO check
build.bat        build and package in one step
fan.bat          the short `fan` alias, written by setup.bat
modules\         PawnIO modules, used to read CPU temperature
data\            frame.png, the log, the remembered profile
src\
  main.rs        commands and the service loop
  config.rs      config parsing and defaults
  sensors.rs     CPU via PawnIO, GPU via NVML or Windows
  render.rs      the styles
  kraken.rs      talking to the cooler
```

The cooler protocol follows [liquidctl](https://github.com/liquidctl/liquidctl)'s
`kraken3` driver. Licensed under the [MIT License](LICENSE).
