@echo off
rem ============================================================
rem  Niuma Timer - shared setup for build.bat / release.bat.
rem  Call from either script (after its setlocal), like:
rem    call "%~dp0common.bat"
rem  common.bat sits next to them, so %~dp0 here == repo root.
rem
rem  Sets: ROOT / SRC / BIN, cargo retry env, and CARGO_BIN with a
rem  PATH fallback for stale terminals (windows opened before the
rem  Rust install never see the updated user PATH).
rem  To force a specific cargo, run first:
rem    set "CARGO_BIN=C:\Users\Tim\.cargo\bin\cargo.exe"
rem  NOTE: no setlocal here - variables must survive the call.
rem ============================================================

set "ROOT=%~dp0"
set "SRC=%ROOT%src-tauri"
set "BIN=%ROOT%bin"

rem Mirror-flaky fallback params (shared). Note: cargo inherits ~/.gitconfig
rem global http.proxy: a global proxy breaks cargo TLS handshake (always under
rem Clash SOCKS5), that proxy now applies only to github.com; cargo reaches
rem rsproxy directly, do NOT switch back to global.
set "CARGO_NET_RETRY=10"
set "CARGO_HTTP_TIMEOUT=180"

if not defined CARGO_BIN set "CARGO_BIN=cargo"
if "%CARGO_BIN%"=="cargo" (
  where cargo >nul 2>&1
  if errorlevel 1 if exist "%USERPROFILE%\.cargo\bin\cargo.exe" set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
)
