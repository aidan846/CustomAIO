@echo off
setlocal EnableExtensions EnableDelayedExpansion

rem CustomAIO fan and pump profiles for liquidctl-supported controllers.
rem setup.bat writes device-specific overrides to data\fan_config.bat.

set "DEVICE_MATCH=Kraken Z"
set "DEVICE_SERIAL="
set "DEVICE_VENDOR="
set "DEVICE_PRODUCT="
set "FAN_CHANNEL=fan"
set "FAN_CONTROL=curve"
set "PUMP_CHANNEL=pump"
set "PUMP_CONTROL=curve"
set "LIQUIDCTL_EXTRA="

set "SILENT_FAN_CURVE=20 30 35 30 40 40 45 55 50 75 55 100"
set "SILENT_PUMP_CURVE=20 60 35 60 40 70 45 80 50 90 55 100"
set "PERF_FAN_CURVE=20 50 35 50 40 60 45 70 50 80 55 90 60 100"
set "PERF_PUMP_CURVE=20 70 35 70 40 80 45 85 50 90 55 95 60 100"
set "SILENT_FAN_FIXED=35"
set "SILENT_PUMP_FIXED=60"
set "PERF_FAN_FIXED=75"
set "PERF_PUMP_FIXED=85"

for /F %%a in ('echo prompt $E ^| cmd') do set "ESC=%%a"
set "RESET=%ESC%[0m"
set "GREEN=%ESC%[92m"
set "YELLOW=%ESC%[93m"
set "ORANGE=%ESC%[38;5;208m"
set "CYAN=%ESC%[96m"
set "WHITE=%ESC%[97m"
set "GRAY=%ESC%[90m"
set "RED=%ESC%[91m"

set "CUSTOMAIO_PYTHON=%~dp0deps\python\Scripts\python.exe"
if not exist "%CUSTOMAIO_PYTHON%" (
    echo %RED%ERROR: CustomAIO dependencies are not installed.%RESET%
    echo Run "%~dp0setup.bat" and choose Install Dependencies.
    exit /b 1
)
if exist "%~dp0data\fan_config.bat" call "%~dp0data\fan_config.bat"

set "PROFILE_DIR=%LOCALAPPDATA%\CustomAIO"
set "PROFILE_FILE=%PROFILE_DIR%\fan-profile.txt"
if not exist "%PROFILE_DIR%" mkdir "%PROFILE_DIR%" >nul 2>&1

if /I "%~1"=="silent" goto silent
if /I "%~1"=="perf" goto performance
if /I "%~1"=="performance" goto performance
if /I "%~1"=="status" goto status
if /I "%~1"=="list" goto list
if /I "%~1"=="init" goto initialize
if /I "%~1"=="setup" goto setup
if /I "%~1"=="help" goto help
goto help

:silent
call :header "Applying Silent profile"
call :set_channel "%PUMP_CHANNEL%" "%PUMP_CONTROL%" "%SILENT_PUMP_FIXED%" "%SILENT_PUMP_CURVE%"
if errorlevel 1 goto error
call :set_channel "%FAN_CHANNEL%" "%FAN_CONTROL%" "%SILENT_FAN_FIXED%" "%SILENT_FAN_CURVE%"
if errorlevel 1 goto error
>"%PROFILE_FILE%" echo Silent
echo.
echo %GREEN%Silent profile applied successfully.%RESET%
echo.
exit /b 0

:performance
call :header "Applying Performance profile"
call :set_channel "%PUMP_CHANNEL%" "%PUMP_CONTROL%" "%PERF_PUMP_FIXED%" "%PERF_PUMP_CURVE%"
if errorlevel 1 goto error
call :set_channel "%FAN_CHANNEL%" "%FAN_CONTROL%" "%PERF_FAN_FIXED%" "%PERF_FAN_CURVE%"
if errorlevel 1 goto error
>"%PROFILE_FILE%" echo Performance
echo.
echo %GREEN%Performance profile applied successfully.%RESET%
echo.
exit /b 0

:status
call :header "Device status"
if exist "%PROFILE_FILE%" (
    set /p ACTIVE_PROFILE=<"%PROFILE_FILE%"
) else (
    set "ACTIVE_PROFILE=Unknown"
)
echo %WHITE%Configured device:%RESET% %DEVICE_MATCH%
echo %WHITE%Last profile:%RESET% !ACTIVE_PROFILE!
echo.
call :liquidctl status
echo.
exit /b %ERRORLEVEL%

:list
"%CUSTOMAIO_PYTHON%" -m liquidctl list -v
exit /b %ERRORLEVEL%

:initialize
call :header "Initializing configured device"
call :liquidctl initialize
exit /b %ERRORLEVEL%

:setup
call "%~dp0setup.bat"
exit /b %ERRORLEVEL%

:help
echo.
echo %CYAN%========================================%RESET%
echo %WHITE%             CustomAIO%RESET%
echo %CYAN%========================================%RESET%
echo.
echo   %GREEN%fan silent%RESET%       Apply the Silent profile
echo   %ORANGE%fan perf%RESET%         Apply the Performance profile
echo   %CYAN%fan status%RESET%       Show status and the last profile
echo   %WHITE%fan init%RESET%         Initialize the configured device
echo   %WHITE%fan list%RESET%         List liquidctl devices
echo   %YELLOW%fan setup%RESET%        Run interactive setup
echo   %GRAY%fan help%RESET%         Show this menu
echo.
exit /b 0

:header
echo.
echo %CYAN%========================================%RESET%
echo %WHITE%             CustomAIO%RESET%
echo %CYAN%========================================%RESET%
echo.
echo %YELLOW%%~1...%RESET%
echo.
exit /b 0

:set_channel
set "CHANNEL=%~1"
set "CONTROL=%~2"
set "FIXED=%~3"
if /I "%CONTROL%"=="disabled" (
    echo %GRAY%Skipping disabled channel "%CHANNEL%".%RESET%
    exit /b 0
)
if /I "%CONTROL%"=="fixed" (
    echo %GRAY%Setting %CHANNEL% to %FIXED%%%.%RESET%
    call :liquidctl set "%CHANNEL%" speed "%FIXED%"
    exit /b %ERRORLEVEL%
)
if /I "%CONTROL%"=="curve" (
    echo %GRAY%Setting %CHANNEL% curve.%RESET%
    call :liquidctl set "%CHANNEL%" speed %~4
    exit /b %ERRORLEVEL%
)
echo %RED%Unknown control type "%CONTROL%" for channel "%CHANNEL%".%RESET%
exit /b 1

:liquidctl
if defined DEVICE_SERIAL (
    "%CUSTOMAIO_PYTHON%" -m liquidctl --serial "%DEVICE_SERIAL%" %LIQUIDCTL_EXTRA% %*
    exit /b %ERRORLEVEL%
)
if defined DEVICE_VENDOR if defined DEVICE_PRODUCT (
    "%CUSTOMAIO_PYTHON%" -m liquidctl --vendor "%DEVICE_VENDOR%" --product "%DEVICE_PRODUCT%" %LIQUIDCTL_EXTRA% %*
    exit /b %ERRORLEVEL%
)
"%CUSTOMAIO_PYTHON%" -m liquidctl --match "%DEVICE_MATCH%" %LIQUIDCTL_EXTRA% %*
exit /b %ERRORLEVEL%

:error
echo.
echo %RED%ERROR: liquidctl could not apply the profile.%RESET%
echo %GRAY%Close vendor control software, check the configured channels in%RESET%
echo %GRAY%%~dp0data\fan_config.bat, and try an elevated terminal if required.%RESET%
echo.
exit /b 1
