@echo off
rem Frontend hot-iteration workflow (NO Rust rebuild per frontend edit).
rem
rem Starts a static server for frontend/ on the devUrl port (1420, see
rem tauri.conf.json) and runs `cargo tauri dev`. The Rust side compiles ONCE;
rem after that, editing index.html / styles.css / js/*.js is just a page
rem reload in the app window - no rebuild at all.
rem
rem Requires: python (for the static server) and tauri-cli (cargo tauri dev).
rem Exit: close the app window, then this script kills the static server.
rem Pure ASCII BY DESIGN and no chcp here - see the header of build.bat:
rem a UTF-8 code page makes child tools mis-decode GBK output (vswhere etc.).
setlocal
rem Shared setup: ROOT/SRC/BIN, cargo PATH fallback, vcvars/RC seeding, and the
rem build-owned TMP that keeps cl.exe from failing with D8050.
call "%~dp0common.bat"
if errorlevel 1 exit /b 1
cd /d "%~dp0"

where python >nul 2>&1
if errorlevel 1 (
  echo [ERROR] python not found - needed for the static frontend server.
  exit /b 1
)

start "niuma-frontend-serve" /min cmd /c "python -m http.server 1420 --bind 127.0.0.1 --directory frontend"

"%CARGO_BIN%" tauri dev
if errorlevel 1 (
  echo [ERROR] cargo tauri dev failed - is tauri-cli installed?
  echo         cargo install tauri-cli --version "^2"
)

taskkill /fi "windowtitle eq niuma-frontend-serve*" >nul 2>&1
endlocal
