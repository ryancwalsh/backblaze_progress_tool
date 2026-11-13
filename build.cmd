@echo off
rem Builds "Backblaze Progress.exe" next to this script. No Administrator rights needed to build.
setlocal
cd /d "%~dp0"

where cargo >nul 2>&1
if errorlevel 1 goto :norust

cargo tauri --version >nul 2>&1
if errorlevel 1 goto :installcli
goto :build

:installcli
echo Installing the Tauri command-line tool. This is a one-time step and takes several minutes...
cargo install tauri-cli --version "^2" --locked
if errorlevel 1 goto :fail

:build
echo Building. The first build downloads and compiles libraries and takes 5-15 minutes...
cd src-tauri
cargo tauri build
if errorlevel 1 goto :fail
cd ..
copy /y "src-tauri\target\release\bz-progress.exe" "Backblaze Progress.exe" >nul
if errorlevel 1 goto :fail
echo.
echo Done: "%~dp0Backblaze Progress.exe"
echo Double-click it and approve the Administrator prompt.
exit /b 0

:norust
echo Rust is not installed or not on PATH. See README.md, section "One-time setup".
exit /b 1

:fail
echo.
echo Build failed. Copy the red error text above into the chat so it can be fixed.
exit /b 1
