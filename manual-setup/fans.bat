@echo off
setlocal EnableExtensions EnableDelayedExpansion

:: ============================================================
:: NZXT Kraken Z53 Fan Control
:: ============================================================

:: Kraken selector
set "KRAKEN=--match Kraken Z"
set "CUSTOMAIO_PYTHON=%~dp0deps\python\Scripts\python.exe"
if not exist "%CUSTOMAIO_PYTHON%" (
    echo CustomAIO dependencies are missing. Keep the deps folder beside fans.bat.
    exit /b 1
)

:: ANSI escape character for colors
for /F %%a in ('echo prompt $E ^| cmd') do set "ESC=%%a"

:: Colors
set "RESET=%ESC%[0m"
set "GREEN=%ESC%[92m"
set "YELLOW=%ESC%[93m"
set "ORANGE=%ESC%[38;5;208m"
set "CYAN=%ESC%[96m"
set "WHITE=%ESC%[97m"
set "GRAY=%ESC%[90m"
set "RED=%ESC%[91m"

:: Store active profile here
set "PROFILE_DIR=%LOCALAPPDATA%\KrakenFan"
set "PROFILE_FILE=%PROFILE_DIR%\profile.txt"

if not exist "%PROFILE_DIR%" mkdir "%PROFILE_DIR%" >nul 2>&1


:: ============================================================
:: Commands
:: ============================================================

if /I "%1"=="silent" goto silent
if /I "%1"=="perf" goto performance
if /I "%1"=="performance" goto performance
if /I "%1"=="status" goto status
if /I "%1"=="help" goto help

goto help


:: ============================================================
:: Silent Profile
:: ============================================================

:silent
echo.
echo %CYAN%========================================%RESET%
echo %WHITE%       NZXT Kraken Z53 Control%RESET%
echo %CYAN%========================================%RESET%
echo.
echo %YELLOW%Applying Silent profile...%RESET%
echo.

echo %GRAY%[1/2] Setting pump curve...%RESET%
"%CUSTOMAIO_PYTHON%" -m liquidctl %KRAKEN% set pump speed 20 60 35 60 40 70 45 80 50 90 55 100

if errorlevel 1 goto error

echo %GRAY%[2/2] Setting fan curve...%RESET%
"%CUSTOMAIO_PYTHON%" -m liquidctl %KRAKEN% set fan speed 20 30 35 30 40 40 45 55 50 75 55 100

if errorlevel 1 goto error

echo Silent>"%PROFILE_FILE%"

echo.
echo %GREEN%Silent profile applied successfully.%RESET%
echo.
echo %WHITE%Pump:%RESET%  60%% at 20-35C  ^>  100%% at 55C
echo %WHITE%Fans:%RESET%  30%% at 20-35C  ^>  100%% at 55C
echo.

exit /b 0


:: ============================================================
:: Performance Profile
:: ============================================================

:performance
echo.
echo %CYAN%========================================%RESET%
echo %WHITE%       NZXT Kraken Z53 Control%RESET%
echo %CYAN%========================================%RESET%
echo.
echo %ORANGE%Applying Performance profile...%RESET%
echo.

echo %GRAY%[1/2] Setting pump curve...%RESET%
"%CUSTOMAIO_PYTHON%" -m liquidctl %KRAKEN% set pump speed 20 70 35 70 40 80 45 85 50 90 55 95 60 100

if errorlevel 1 goto error

echo %GRAY%[2/2] Setting fan curve...%RESET%
"%CUSTOMAIO_PYTHON%" -m liquidctl %KRAKEN% set fan speed 20 50 35 50 40 60 45 70 50 80 55 90 60 100

if errorlevel 1 goto error

echo Performance>"%PROFILE_FILE%"

echo.
echo %GREEN%Performance profile applied successfully.%RESET%
echo.
echo %WHITE%Pump:%RESET%  70%% at 20-35C  ^>  100%% at 60C
echo %WHITE%Fans:%RESET%  50%% at 20-35C  ^>  100%% at 60C
echo.

exit /b 0


:: ============================================================
:: Status
:: ============================================================

:status
echo.
echo %CYAN%========================================%RESET%
echo %WHITE%         Kraken Z53 Status%RESET%
echo %CYAN%========================================%RESET%
echo.

if exist "%PROFILE_FILE%" (
    set /p ACTIVE_PROFILE=<"%PROFILE_FILE%"

    if /I "!ACTIVE_PROFILE!"=="Silent" (
        echo %WHITE%Active Profile:%RESET% %GREEN%!ACTIVE_PROFILE!%RESET%
    ) else (
        echo %WHITE%Active Profile:%RESET% %ORANGE%!ACTIVE_PROFILE!%RESET%
    )
) else (
    echo %WHITE%Active Profile:%RESET% %GRAY%Unknown%RESET%
)

echo.
"%CUSTOMAIO_PYTHON%" -m liquidctl %KRAKEN% status
echo.

exit /b 0


:: ============================================================
:: Help
:: ============================================================

:help
echo.
echo %CYAN%========================================%RESET%
echo %WHITE%       NZXT Kraken Z53 Control%RESET%
echo %CYAN%========================================%RESET%
echo.
echo %WHITE%Commands:%RESET%
echo.
echo   %GREEN%fan silent%RESET%     Apply Silent profile
echo   %ORANGE%fan perf%RESET%       Apply Performance profile
echo   %CYAN%fan status%RESET%     Show active profile and cooler status
echo   %GRAY%fan help%RESET%       Show this menu
echo.
exit /b 0


:: ============================================================
:: Error
:: ============================================================

:error
echo.
echo %RED%ERROR: Failed to communicate with the Kraken Z53.%RESET%
echo.
echo %GRAY%Make sure NZXT CAM is completely closed.%RESET%
echo %GRAY%You can also try running Command Prompt as Administrator.%RESET%
echo.
exit /b 1
