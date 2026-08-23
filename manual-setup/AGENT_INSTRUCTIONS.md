# Manual CustomAIO conversion

Use this kit only when the user explicitly requests the older manual Kraken Z53 workflow. Explain that it replaces the normal setup files and get confirmation before deleting or moving anything.

1. Have the user run the parent `setup.bat` first; keep `CustomAIO\deps` because it contains the local Python environment and LibreHardwareMonitor DLLs.
2. Back up the parent folder, then remove its existing files and `data` folder.
3. Copy `manual-setup\lcd.py` and `manual-setup\fans.bat` to the parent folder. Do not remove `manual-setup` unless the user requests it.
4. Do not install packages globally. Test with `deps\python\Scripts\python.exe lcd.py`.

The scripts use paths relative to their location. The manual `lcd.py` loads `deps\lhm\LibreHardwareMonitorLib.dll`, logs to `CustomAIO\log.txt`, and must show unavailable temperatures as `N/A` rather than inventing a value.
