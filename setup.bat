@echo off
setlocal EnableExtensions EnableDelayedExpansion

rem CustomAIO interactive installer. Administrator rights are required for
rem device access setup, dependency installation, and scheduled-task creation.

fltmc >nul 2>&1
if not "%ERRORLEVEL%"=="0" (
    echo Requesting Administrator access...
    set "CUSTOMAIO_SETUP=%~f0"
    set "CUSTOMAIO_DIR=%~dp0"
    powershell -NoProfile -Command "Start-Process -FilePath $env:CUSTOMAIO_SETUP -WorkingDirectory $env:CUSTOMAIO_DIR -Verb RunAs"
    exit /b
)

cd /d "%~dp0"
title CustomAIO Setup
for /F %%a in ('echo prompt $E ^| cmd') do set "ESC=%%a"
set "RESET=%ESC%[0m"
set "CYAN=%ESC%[96m"
set "GREEN=%ESC%[92m"
set "YELLOW=%ESC%[93m"
set "RED=%ESC%[91m"
set "GRAY=%ESC%[90m"

set "PYTHON_EXE="
for /f "delims=" %%P in ('where python 2^>nul') do if not defined PYTHON_EXE set "PYTHON_EXE=%%P"
if not defined PYTHON_EXE (
    echo.
    echo ERROR: Python was not found on PATH.
    echo Install Python 3, enable "Add python.exe to PATH", then run setup.bat again.
    pause
    exit /b 1
)

"%PYTHON_EXE%" --version >nul 2>&1
if errorlevel 1 (
    echo ERROR: The Python command could not be started: %PYTHON_EXE%
    pause
    exit /b 1
)

"%PYTHON_EXE%" -c "import sys; raise SystemExit(0 if sys.version_info >= (3, 10) else 1)" >nul 2>&1
if errorlevel 1 (
    echo ERROR: CustomAIO requires Python 3.10 or newer.
    echo Install a supported Python version, enable its PATH option, then run setup.bat again.
    pause
    exit /b 1
)

set "SYSTEM_PYTHON=%PYTHON_EXE%"
set "VENV_DIR=%~dp0deps\python"
set "VENV_PYTHON=%VENV_DIR%\Scripts\python.exe"
set "VENV_PYTHONW=%VENV_DIR%\Scripts\pythonw.exe"
if not exist "%VENV_PYTHON%" (
    echo Creating CustomAIO's isolated Python environment...
    "%SYSTEM_PYTHON%" -m venv "%VENV_DIR%"
    if errorlevel 1 goto dependency_error
)
set "PYTHON_EXE=%VENV_PYTHON%"
set "PYTHONW_EXE=%VENV_PYTHONW%"


cls
echo.
echo %CYAN%================================================%RESET%
echo %GREEN%                 CustomAIO Setup%RESET%
echo %CYAN%================================================%RESET%
echo.
echo %GRAY%System Python:%RESET% %SYSTEM_PYTHON%
echo %GRAY%Project dependencies:%RESET% %VENV_DIR%
echo.
echo %GREEN%[1]%RESET% Fan / pump controller
echo %GREEN%[2]%RESET% LCD temperature display
echo %GREEN%[3]%RESET% Configure both
echo.
echo %YELLOW%[4]%RESET% Repair or install project dependencies
echo %RED%[U]%RESET% Uninstall CustomAIO
echo %GRAY%[Q] Quit%RESET%
echo.
echo %GRAY%Press Enter to choose the recommended option shown in [brackets].%RESET%
echo.
set /p "SETUP_CHOICE=Choose an option [3]: "
if not defined SETUP_CHOICE set "SETUP_CHOICE=3"

if /I "%SETUP_CHOICE%"=="Q" exit /b 0
if /I "%SETUP_CHOICE%"=="U" goto uninstall
if "%SETUP_CHOICE%"=="1" set "DO_FAN=1"
if "%SETUP_CHOICE%"=="2" set "DO_LCD=1"
if "%SETUP_CHOICE%"=="3" (
    set "DO_FAN=1"
    set "DO_LCD=1"
)
if "%SETUP_CHOICE%"=="4" set "DEPS_ONLY=1"
if not defined DO_FAN if not defined DO_LCD if not defined DEPS_ONLY (
    echo Invalid choice.
    pause
    exit /b 1
)

echo.
echo Installing/upgrading dependencies in CustomAIO\deps\python...
"%PYTHON_EXE%" -m pip install --upgrade pip
if errorlevel 1 goto dependency_error
"%PYTHON_EXE%" -m pip install --upgrade -r "%~dp0deps\requirements.txt"
if errorlevel 1 goto dependency_error

if defined DEPS_ONLY goto dependencies_complete

if not exist "%~dp0lcd.py" (
    echo ERROR: lcd.py must be in the same folder as setup.bat.
    goto setup_error
)
if defined DO_FAN if not exist "%~dp0fan.bat" (
    echo ERROR: fan.bat must be in the same folder as setup.bat.
    goto setup_error
)

if defined DO_FAN call :configure_fan
if errorlevel 1 goto setup_error
if defined DO_LCD call :configure_lcd
if errorlevel 1 goto setup_error

echo.
set /p "ADD_PATH=Add the CustomAIO folder (%~dp0) to your user PATH? [Y/n]: "
if /I not "!ADD_PATH!"=="N" call :add_to_path

echo.
echo Setup complete.
if defined DO_FAN echo Try: fan status
if defined DO_LCD echo LCD log: %~dp0data\lcd.log
echo.
pause
exit /b 0

:dependencies_complete
echo.
echo Dependencies installed successfully.
echo.
pause
exit /b 0

:configure_fan
cls
echo.
echo ========================================
echo          Fan controller setup
echo ========================================
echo.
"%PYTHON_EXE%" "%~dp0lcd.py" --list-devices
if errorlevel 1 exit /b 1
set "FAN_DEFAULT=1"
for /f "delims=" %%D in ('call "%PYTHON_EXE%" "%~dp0lcd.py" --recommended-device fan') do set "FAN_DEFAULT=%%D"
echo.
set /p "FAN_DEVICE=Choose the fan/controller device number [!FAN_DEFAULT!]: "
if not defined FAN_DEVICE set "FAN_DEVICE=!FAN_DEFAULT!"

echo.
echo Channel names vary by device. Common examples are fan, fans, fan1,
echo pump, sync, or fan1 through fan8. Check the linked liquidctl device guide
echo if the detected channel hint above is empty.
echo.
set /p "FAN_CHANNEL=Fan channel [fan; type none to disable]: "
if not defined FAN_CHANNEL set "FAN_CHANNEL=fan"
if /I "!FAN_CHANNEL!"=="none" (
    set "FAN_CHANNEL=fan"
    set "FAN_CONTROL=disabled"
) else (
    set /p "FAN_CONTROL=Fan control type [curve/fixed, default curve]: "
    if not defined FAN_CONTROL set "FAN_CONTROL=curve"
)

set /p "PUMP_CHANNEL=Pump channel [pump; type none to disable]: "
if not defined PUMP_CHANNEL set "PUMP_CHANNEL=pump"
if /I "!PUMP_CHANNEL!"=="none" (
    set "PUMP_CHANNEL=pump"
    set "PUMP_CONTROL=disabled"
) else (
    set /p "PUMP_CONTROL=Pump control type [curve/fixed, default curve]: "
    if not defined PUMP_CONTROL set "PUMP_CONTROL=curve"
)

"%PYTHON_EXE%" "%~dp0lcd.py" --configure-fan "!FAN_DEVICE!" --fan-channel "!FAN_CHANNEL!" --fan-control "!FAN_CONTROL!" --pump-channel "!PUMP_CHANNEL!" --pump-control "!PUMP_CONTROL!"
if errorlevel 1 exit /b 1

echo.
set /p "INITIALIZE_DEVICE=Initialize this device now? [Y/n]: "
if /I not "!INITIALIZE_DEVICE!"=="N" call "%~dp0fan.bat" init
exit /b %ERRORLEVEL%

:configure_lcd
cls
echo.
echo ========================================
echo           LCD controller setup
echo ========================================
echo.
"%PYTHON_EXE%" "%~dp0lcd.py" --list-devices
if errorlevel 1 exit /b 1
set "LCD_DEFAULT=1"
for /f "delims=" %%D in ('call "%PYTHON_EXE%" "%~dp0lcd.py" --recommended-device lcd') do set "LCD_DEFAULT=%%D"
echo.
echo Choose only a device whose liquidctl guide supports static LCD images.
set /p "LCD_DEVICE=Choose the LCD device number [!LCD_DEFAULT!]: "
if not defined LCD_DEVICE set "LCD_DEVICE=!LCD_DEFAULT!"

for /f "usebackq delims=" %%G in (`powershell -NoProfile -Command "(Get-CimInstance Win32_VideoController | Select-Object -ExpandProperty Name) -join ', '"`) do set "DETECTED_GPU=%%G"
for /f "usebackq delims=" %%C in (`powershell -NoProfile -Command "(Get-CimInstance Win32_Processor | Select-Object -ExpandProperty Name) -join ', '"`) do set "DETECTED_CPU=%%C"

echo.
"%PYTHON_EXE%" -c "from pathlib import Path; p = Path(r'%~dp0deps\lhm'); assert (p / 'LibreHardwareMonitorLib.dll').exists() and (p / 'RAMSPDToolkit-NDD.dll').exists(); import pythonnet; pythonnet.load('coreclr'); import clr; clr.AddReference(str(p / 'LibreHardwareMonitorLib.dll'))" >nul 2>&1
if errorlevel 1 (
    echo WARNING: The bundled LibreHardwareMonitor files could not be loaded.
    echo CPU and AMD/Intel GPU temperatures will show N/A until deps\lhm is repaired.
) else (
    echo Bundled LibreHardwareMonitor found. CPU and supported GPU temperatures
    echo can be read without installing a separate monitoring application.
)

echo.
echo %YELLOW%PawnIO enables low-level CPU temperature access for LibreHardwareMonitor.%RESET%
echo %GRAY%It installs a Windows driver and opens the official installer visibly.%RESET%
set /p "INSTALL_PAWNIO=Install or update PawnIO now [Y/n]: "
if /I not "!INSTALL_PAWNIO!"=="N" call :install_pawnio
if errorlevel 1 (
    echo %YELLOW%PawnIO installation did not complete. CPU temperature may remain N/A.%RESET%
    pause
)

echo.
echo Detected GPU: !DETECTED_GPU!
echo [1] NVIDIA GPU ^(nvidia-smi^)
echo [2] Bundled LibreHardwareMonitor ^(AMD Radeon, Intel, or NVIDIA fallback^)
echo [3] No GPU temperature
set "GPU_DEFAULT=3"
for /f %%D in ('powershell -NoProfile -Command "$n = (Get-CimInstance Win32_VideoController).Name -join ' '; if ($n -match 'NVIDIA') { 1 } elseif ($n -match 'AMD|Radeon|Intel') { 2 } else { 3 }"') do set "GPU_DEFAULT=%%D"
set /p "GPU_CHOICE=Choose GPU temperature source [!GPU_DEFAULT!]: "
if not defined GPU_CHOICE set "GPU_CHOICE=!GPU_DEFAULT!"
if "!GPU_CHOICE!"=="1" (
    set "GPU_SOURCE=nvidia-smi"
    set "GPU_VENDOR=nvidia"
) else if "!GPU_CHOICE!"=="2" (
    set "GPU_SOURCE=librehardwaremonitor"
    set "GPU_VENDOR=auto"
) else (
    set "GPU_SOURCE=none"
    set "GPU_VENDOR=auto"
)

echo.
echo Detected CPU: !DETECTED_CPU!
echo [1] Bundled LibreHardwareMonitor ^(recommended for Intel and AMD Ryzen^)
echo [2] No CPU temperature
set "CPU_DEFAULT=1"
set /p "CPU_CHOICE=Choose CPU temperature source [!CPU_DEFAULT!]: "
if not defined CPU_CHOICE set "CPU_CHOICE=!CPU_DEFAULT!"
if "!CPU_CHOICE!"=="1" (
    set "CPU_SOURCE=librehardwaremonitor"
    set "CPU_VENDOR=auto"
) else (
    set "CPU_SOURCE=none"
    set "CPU_VENDOR=auto"
)

"%PYTHON_EXE%" "%~dp0lcd.py" --configure "!LCD_DEVICE!" --cpu-source "!CPU_SOURCE!" --cpu-vendor "!CPU_VENDOR!" --gpu-source "!GPU_SOURCE!" --gpu-vendor "!GPU_VENDOR!"
if errorlevel 1 exit /b 1

if not exist "%PYTHONW_EXE%" (
    echo.
    echo WARNING: pythonw.exe was not found beside python.exe:
    echo %PYTHONW_EXE%
    echo LCD configuration was saved, but automatic startup cannot be installed.
    exit /b 0
)

echo.
set /p "AUTO_START=Run the LCD invisibly at logon using Task Scheduler? [Y/n]: "
if /I "!AUTO_START!"=="N" exit /b 0
call :install_task
if errorlevel 1 exit /b 1

set /p "RUN_TASK=Start the LCD task now? [Y/n]: "
if /I not "!RUN_TASK!"=="N" powershell -NoProfile -Command "Start-ScheduledTask -TaskName 'CustomAIO LCD'"
exit /b %ERRORLEVEL%

:install_task
set "CUSTOMAIO_PYTHONW=%PYTHONW_EXE%"
set "CUSTOMAIO_LCD=%~dp0lcd.py"
set "CUSTOMAIO_DIR=%~dp0"
powershell -NoProfile -Command "$action = New-ScheduledTaskAction -Execute $env:CUSTOMAIO_PYTHONW -Argument ([char]34 + $env:CUSTOMAIO_LCD + [char]34) -WorkingDirectory $env:CUSTOMAIO_DIR; $trigger = New-ScheduledTaskTrigger -AtLogOn -User ($env:USERDOMAIN + '\' + $env:USERNAME); $settings = New-ScheduledTaskSettingsSet -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) -MultipleInstances IgnoreNew -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries; $principal = New-ScheduledTaskPrincipal -UserId ($env:USERDOMAIN + '\' + $env:USERNAME) -LogonType Interactive -RunLevel Highest; Register-ScheduledTask -TaskName 'CustomAIO LCD' -Action $action -Trigger $trigger -Settings $settings -Principal $principal -Description 'Updates a liquidctl-supported AIO LCD at user logon.' -Force | Out-Null"
if errorlevel 1 exit /b 1
echo Scheduled task "CustomAIO LCD" installed or updated.
exit /b 0

:add_to_path
set "CUSTOMAIO_DIR=%~dp0"
powershell -NoProfile -Command "$dir = $env:CUSTOMAIO_DIR.TrimEnd('\'); $current = [Environment]::GetEnvironmentVariable('Path', 'User'); $parts = @($current -split ';' | Where-Object { $_ }); if ($parts.TrimEnd('\') -notcontains $dir) { $newPath = (($parts + $dir) -join ';'); [Environment]::SetEnvironmentVariable('Path', $newPath, 'User'); Write-Host 'Added the CustomAIO folder to user PATH. Open a new terminal before using fan.' } else { Write-Host 'The CustomAIO folder is already on user PATH.' }"
exit /b %ERRORLEVEL%

:install_pawnio
set "CUSTOMAIO_PAWNIO_DIR=%~dp0deps\installer"
set "CUSTOMAIO_PAWNIO_EXE=%CUSTOMAIO_PAWNIO_DIR%\PawnIO_setup.exe"
if not exist "%CUSTOMAIO_PAWNIO_DIR%" mkdir "%CUSTOMAIO_PAWNIO_DIR%" >nul 2>&1
echo.
echo Downloading the official PawnIO installer...
powershell -NoProfile -Command "Invoke-WebRequest -Uri 'https://github.com/namazso/PawnIO.Setup/releases/latest/download/PawnIO_setup.exe' -OutFile $env:CUSTOMAIO_PAWNIO_EXE"
if errorlevel 1 exit /b 1
echo Launching PawnIO installer. Complete its visible prompts, then close it.
powershell -NoProfile -Command "$process = Start-Process -FilePath $env:CUSTOMAIO_PAWNIO_EXE -Wait -PassThru; exit $process.ExitCode"
if errorlevel 1 exit /b 1
echo PawnIO installer closed. Press any key to continue setup.
pause >nul
exit /b 0

:uninstall
cls
echo.
echo %RED%CustomAIO uninstaller%RESET%
echo.
echo This removes the logon task, local dependencies, settings, and this CustomAIO folder.
set /p "UNINSTALL_CONFIRM=Type REMOVE to continue: "
if /I not "%UNINSTALL_CONFIRM%"=="REMOVE" exit /b 0
for %%I in ("%~dp0.") do set "CUSTOMAIO_REMOVE_DIR=%%~fI"
if not exist "%CUSTOMAIO_REMOVE_DIR%\setup.bat" (
    echo %RED%Safety check failed: setup.bat is missing from "%CUSTOMAIO_REMOVE_DIR%".%RESET%
    pause
    exit /b 1
)
if not exist "%CUSTOMAIO_REMOVE_DIR%\lcd.py" (
    echo %RED%Safety check failed: lcd.py is missing from "%CUSTOMAIO_REMOVE_DIR%".%RESET%
    pause
    exit /b 1
)
schtasks /Delete /TN "CustomAIO LCD" /F >nul 2>&1
set "CUSTOMAIO_REMOVE_HELPER=%TEMP%\CustomAIO-uninstall-%RANDOM%.cmd"
>"%CUSTOMAIO_REMOVE_HELPER%" echo @echo off
>>"%CUSTOMAIO_REMOVE_HELPER%" echo timeout /t 2 /nobreak ^>nul
>>"%CUSTOMAIO_REMOVE_HELPER%" echo rmdir /s /q "%CUSTOMAIO_REMOVE_DIR%"
start "" /b cmd /c ""%CUSTOMAIO_REMOVE_HELPER%""
exit /b 0

:dependency_error
echo.
echo ERROR: Dependency installation failed.
echo Check the internet connection and pip output above, then run setup.bat again.
pause
exit /b 1

:setup_error
echo.
echo ERROR: Setup did not finish. Review the output above.
pause
exit /b 1
