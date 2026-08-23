Manual Setup Kit
================

Compatibility disclaimer: this project has only been tested on the author's
single computer. It may need adjustment for your cooler, hardware, or Windows
installation.

Use this only if you want the older manual Kraken Z53 workflow. First run the
parent folder's setup.bat once so deps\python and deps\lhm are available.

Steps
-----
1. Back up the parent CustomAIO folder.
2. In that folder, remove the existing files and the data folder, but keep deps.
3. Copy this folder's lcd.py and fans.bat into the parent folder.

Run the LCD manually with:

  deps\python\Scripts\python.exe lcd.py

Run fan profiles with:

  fans.bat silent
  fans.bat perf

LCD Task Scheduler entry
------------------------
Create Task in Task Scheduler (not Basic Task), then use:

  General: Run only when user is logged on; check Run with highest privileges.
  Trigger: At log on, for your user account.
  Action - Program/script:

    <CustomAIO folder>\deps\python\Scripts\pythonw.exe

  Action - Add arguments:

    "<CustomAIO folder>\lcd.py"

  Action - Start in:

    <CustomAIO folder>

`pythonw.exe` keeps the LCD service invisible. Use the visible python.exe command
above first if you need to troubleshoot it.

The copied scripts use only paths relative to their folder. lcd.py writes
log.txt and lcd_frame.png beside itself. Close NZXT CAM or other software that
is controlling the cooler first.
