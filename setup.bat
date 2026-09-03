@echo off
setlocal EnableExtensions EnableDelayedExpansion

rem CustomAIO setup: PATH entry, logon task for the LCD service, PawnIO check.
rem Right-click this file and choose "Run as administrator".

set "HERE=%~dp0"
if "%HERE:~-1%"=="\" set "HERE=%HERE:~0,-1%"
set "EXE=%HERE%\fan.exe"
set "TASK=CustomAIO LCD"

for /F %%a in ('echo prompt $E ^| cmd') do set "ESC=%%a"
set "RESET=%ESC%[0m"
set "GREEN=%ESC%[92m"
set "YELLOW=%ESC%[93m"
set "CYAN=%ESC%[96m"
set "WHITE=%ESC%[97m"
set "GRAY=%ESC%[90m"
set "RED=%ESC%[91m"

net session >nul 2>&1
if errorlevel 1 (
    echo %RED%This script must run as administrator.%RESET%
    echo %GRAY%Right-click setup.bat and choose "Run as administrator".%RESET%
    echo.
    pause
    exit /b 1
)

rem Use the freshly built binary if the release copy is newer or the top-level
rem one is missing, so `cargo build --release` followed by setup just works.
if exist "%HERE%\target\release\fan.exe" (
    if not exist "%EXE%" (
        copy /Y "%HERE%\target\release\fan.exe" "%EXE%" >nul
    ) else (
        for %%A in ("%HERE%\target\release\fan.exe") do set "NEWTS=%%~tA"
        for %%B in ("%EXE%") do set "OLDTS=%%~tB"
        if not "!NEWTS!"=="!OLDTS!" copy /Y "%HERE%\target\release\fan.exe" "%EXE%" >nul
    )
)

if not exist "%EXE%" (
    echo %RED%fan.exe was not found.%RESET%
    echo %GRAY%Build it first:  cargo build --release%RESET%
    echo.
    pause
    exit /b 1
)

:menu
cls
echo.
echo %CYAN%========================================%RESET%
echo %WHITE%          CustomAIO setup%RESET%
echo %CYAN%========================================%RESET%
echo.
echo   %WHITE%1%RESET%  Full setup ^(PATH + LCD at logon^)
echo   %WHITE%2%RESET%  Add this folder to PATH only
echo   %WHITE%3%RESET%  Create/replace the LCD logon task only
echo   %WHITE%4%RESET%  Remove the LCD task
echo   %WHITE%5%RESET%  Check PawnIO ^(needed for CPU temperature^)
echo   %WHITE%6%RESET%  Show detected coolers
echo   %GRAY%0  Exit%RESET%
echo.
set "CHOICE="
set /p "CHOICE=Select: "
if "%CHOICE%"=="1" call :do_path & call :do_task & goto done
if "%CHOICE%"=="2" call :do_path & goto done
if "%CHOICE%"=="3" call :do_task & goto done
if "%CHOICE%"=="4" call :remove_task & goto done
if "%CHOICE%"=="5" call :check_pawnio & goto done
if "%CHOICE%"=="6" "%EXE%" devices & goto done
if "%CHOICE%"=="0" exit /b 0
goto menu

:done
echo.
pause
goto menu

rem ----------------------------------------------------------
:do_path
echo.
echo %YELLOW%Adding CustomAIO to your PATH...%RESET%
rem Read the user PATH from the registry rather than %PATH%, which is the
rem already-merged system+user value.
set "USERPATH="
for /F "tokens=2,*" %%A in ('reg query "HKCU\Environment" /v Path 2^>nul ^| find "Path"') do set "USERPATH=%%B"
echo !USERPATH! | find /I "%HERE%" >nul
if not errorlevel 1 (
    echo %GRAY%Already on PATH.%RESET%
    exit /b 0
)
if defined USERPATH (
    setx PATH "!USERPATH!;%HERE%" >nul
) else (
    setx PATH "%HERE%" >nul
)
echo %GREEN%Added.%RESET% %GRAY%Open a new terminal, then try: fan status%RESET%
exit /b 0

rem ----------------------------------------------------------
:do_task
echo.
rem Only one program may drive the cooler, so this deliberately takes over the
rem existing "CustomAIO LCD" task rather than adding a second one. Any current
rem task is exported first so it can be restored.
schtasks /Query /TN "%TASK%" >nul 2>&1
if not errorlevel 1 (
    echo %YELLOW%An existing "%TASK%" task was found - it will be replaced.%RESET%
    if not exist "%HERE%\data" mkdir "%HERE%\data" >nul 2>&1
    set "BACKUP=%HERE%\data\old-lcd-task.xml"
    schtasks /Query /TN "%TASK%" /XML >"!BACKUP!" 2>nul
    if exist "!BACKUP!" (
        echo %GRAY%Backed up to data\old-lcd-task.xml%RESET%
        echo %GRAY%Restore it later with:%RESET%
        echo %GRAY%  schtasks /Create /TN "%TASK%" /XML "!BACKUP!" /F%RESET%
    )
    echo.
    set "GO="
    set /p "GO=Replace the existing task? [y/N] "
    if /I not "!GO!"=="y" (
        echo %GRAY%Left the existing task alone.%RESET%
        exit /b 0
    )
    schtasks /End /TN "%TASK%" >nul 2>&1
)
echo.
echo %YELLOW%Creating the "%TASK%" logon task...%RESET%
rem Runs in the logged-on session with highest privileges: PawnIO needs the
rem elevation, and the GPU sensors need a real user session.
powershell -NoProfile -ExecutionPolicy Bypass -Command ^
  "$ErrorActionPreference='Stop';" ^
  "$a=New-ScheduledTaskAction -Execute '%EXE%' -Argument 'lcd --quiet' -WorkingDirectory '%HERE%';" ^
  "$t=New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME;" ^
  "$p=New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive -RunLevel Highest;" ^
  "$s=New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1);" ^
  "Register-ScheduledTask -TaskName '%TASK%' -Action $a -Trigger $t -Principal $p -Settings $s -Force | Out-Null;" ^
  "Start-ScheduledTask -TaskName '%TASK%';"
if errorlevel 1 (
    echo %RED%Could not create the task.%RESET%
    exit /b 1
)
echo %GREEN%Task created and started.%RESET%
echo %GRAY%Close NZXT CAM and any other CustomAIO service so they don't fight over the LCD.%RESET%
exit /b 0

rem ----------------------------------------------------------
:remove_task
echo.
echo %YELLOW%Removing the "%TASK%" task...%RESET%
schtasks /End /TN "%TASK%" >nul 2>&1
schtasks /Delete /TN "%TASK%" /F >nul 2>&1
if errorlevel 1 (
    echo %GRAY%No such task.%RESET%
) else (
    echo %GREEN%Removed.%RESET%
)
exit /b 0

rem ----------------------------------------------------------
:check_pawnio
echo.
if exist "%ProgramFiles%\PawnIO\PawnIOLib.dll" (
    echo %GREEN%PawnIO is installed.%RESET%
    sc query PawnIO | find "RUNNING" >nul
    if errorlevel 1 (
        echo %YELLOW%The PawnIO driver is not running.%RESET%
    ) else (
        echo %GRAY%Driver is running.%RESET%
    )
) else (
    echo %YELLOW%PawnIO is not installed.%RESET%
    echo %GRAY%It provides CPU temperature. Without it the LCD shows N/A for CPU.%RESET%
    echo %GRAY%Download it from https://pawnio.eu and run setup again.%RESET%
)
echo.
echo %GRAY%Reading CPU temperature also requires this program to run elevated,%RESET%
echo %GRAY%which the logon task above already does.%RESET%
exit /b 0
