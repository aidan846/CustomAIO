@echo off
setlocal EnableExtensions

rem Builds CustomAIO and assembles the shareable package in one step.
rem Equivalent to `cargo dist`; double-click this or run it from a terminal.
rem
rem Plain `cargo build --release` only produces customaio.exe - Cargo has no
rem post-build hook, so the packaging step has to be invoked separately.

cd /d "%~dp0"

where cargo >nul 2>&1
if errorlevel 1 (
    echo ERROR: cargo was not found on PATH.
    echo Install Rust from https://rustup.rs and reopen this window.
    echo.
    pause
    exit /b 1
)

echo Building...
cargo build --release
if errorlevel 1 (
    echo.
    echo Build failed.
    echo.
    pause
    exit /b 1
)

rem The freshly built binary packages itself.
"%~dp0target\release\customaio.exe" package
if errorlevel 1 (
    echo.
    echo Packaging failed.
    echo.
    pause
    exit /b 1
)

echo.
pause
