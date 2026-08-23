# CustomAIO

> **Compatibility disclaimer:** this project has only been tested on my single computer. It may need adjustment for your cooler, hardware, or Windows installation.

CustomAIO controls fan/pump profiles and an AIO LCD on Windows. It uses [**LibreHardwareMonitor**](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor) for local CPU/GPU temperature readings and [**liquidctl**](https://github.com/liquidctl/liquidctl) to discover and control supported coolers.

## Install

1. Install Python 3.10 or later, selecting **Add Python to PATH**.
2. Download or clone anywhere you like - I highly recommend into C:/Users/YOURNAME/Scripts (make a folder)
4. Right-click `setup.bat` and choose **Run as administrator**.
5. Select the fan/pump, LCD, or both setup option and follow the prompts, you can usually just press enter for everything.

Close NZXT CAM or other vendor software that controls the cooler before running CustomAIO.

Setup creates a project-local Python environment in `deps\python`, installs required packages, saves settings in `data`, and can create an LCD task that starts at logon. It does not install Python packages globally.

## Use

After setup, open a new Command Prompt if you chose to add CustomAIO to PATH (recommended) :

```cmd
fan silent
fan perf
fan status
```

To test the LCD visibly, run:

```cmd
deps\python\Scripts\python.exe lcd.py
```

There is no point in doing this if you chose to set up Task Scheduler. As you should stop the task first.

The LCD image and log are written to `data\lcd_frame.png` and `data\lcd.log`.

## Notes

- Fan/pump control requires a liquidctl-supported speed channel.
- LCD output requires a liquidctl-supported static-image LCD.
- During LCD setup, CustomAIO can download and run the official PawnIO installer; some systems need it for CPU temperature readings.
- If a sensor is unavailable, the LCD shows `N/A`. Run setup again and choose **Repair dependencies** if required files are missing.

The optional `manual-setup` folder is for experienced users who explicitly want the older Kraken Z53-style manual workflow. Read its instructions before changing files.

Also feel free to provide any files, specifically the manual-setup for the Kraken Z53 to an AI Agent and have it tailor it to your needs.

CustomAIO is distributed under the [MIT License](LICENSE).
