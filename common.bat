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

rem release builds embed version/icon resources via rc.exe (Windows Kits).
rem Plain terminals lack it in PATH; probe the newest SDK copy and pin RC
rem (embed_resource honors $RC before $PATH). Override: set "RC=<full path>".
rem Self-heal: a stale RC left in the terminal pointing to a removed file
rem (SDK upgrade / machine switch) makes embed_resource fail with "RC.EXE
rem not set" - treat it as unset and re-probe. Keep this file ASCII-only:
rem cmd mis-parses multibyte comments here (no chcp guard of its own).
if defined RC if not exist "%RC%" set "RC="
if not defined RC (
  for /f "delims=" %%d in ('dir /b /ad /o-n "%ProgramFiles(x86)%\Windows Kits\10\bin" 2^>nul') do (
    if not defined RC if exist "%ProgramFiles(x86)%\Windows Kits\10\bin\%%d\x64\rc.exe" set "RC=%ProgramFiles(x86)%\Windows Kits\10\bin\%%d\x64\rc.exe"
  )
)
