# CustomAIO

> **Compatibility disclaimer:** this project has only been tested on the author's single computer. It may need adjustment for your cooler, hardware, or Windows installation.

CustomAIO controls fan/pump profiles and an AIO LCD on Windows. It uses [**LibreHardwareMonitor**](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor) for local CPU/GPU temperature readings and [**liquidctl**](https://github.com/liquidctl/liquidctl) to discover and control supported coolers.

## Install

1. Install Python 3.10 or later, selecting **Add Python to PATH**.
2. Download or clone this project anywhere you like.
3. Right-click `setup.bat` and choose **Run as administrator**.
4. Select the fan/pump, LCD, or both setup option and follow the prompts.

Setup creates a project-local Python environment in `deps\python`, installs required packages, saves settings in `data`, and can create an LCD task that starts at logon. It does not install Python packages globally.

Close NZXT CAM or other vendor software that controls the cooler before running CustomAIO.

## Use

After setup, open a new Command Prompt if you chose to add CustomAIO to PATH:

```cmd
fan silent
fan perf
fan status
fan init
fan setup
```

To test the LCD visibly, run:

```cmd
deps\python\Scripts\python.exe lcd.py
```

The LCD image and log are written to `data\lcd_frame.png` and `data\lcd.log`.

## Notes

- Fan/pump control requires a liquidctl-supported speed channel.
- LCD output requires a liquidctl-supported static-image LCD.
- During LCD setup, CustomAIO can download and run the official PawnIO installer; some systems need it for CPU temperature readings.
- If a sensor is unavailable, the LCD shows `N/A`. Run setup again and choose **Repair dependencies** if required files are missing.

The optional `manual-setup` folder is for experienced users who explicitly want the older Kraken Z53-style manual workflow. Read its instructions before changing files.

CustomAIO is distributed under the [MIT License](LICENSE).
