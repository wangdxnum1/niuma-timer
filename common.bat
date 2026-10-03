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

rem Build-owned temp dir (2026-10-02 cl.exe D8050 post-mortem). Whenever
rem debug info is on (-Z7, which cc-rs passes for BOTH profiles) cl.exe
rem writes a debug record holding its command line into %TMP%. If %TMP%
rem is missing, unwritable or unset, cl aborts with D8050 (exit 2) BEFORE
rem c1.dll / c1xx.dll runs - so every C dependency (ring, libsqlite3-sys,
rem vswhom-sys) fails at once and the toolchain looks broken. Ambient TMP
rem belongs to the caller (sandboxed / redirected shells hand us whatever
rem their parent had), so create and pin our own instead of trusting it.
if not exist "%ROOT%.tmp" mkdir "%ROOT%.tmp" >nul 2>&1
if not exist "%ROOT%.tmp\" (
  echo [ERROR] cannot create build temp dir "%ROOT%.tmp"
  exit /b 1
)
set "TMP=%ROOT%.tmp"
set "TEMP=%ROOT%.tmp"
rem Fail fast with a readable message instead of a cryptic D8050 from cl.
echo ok > "%TMP%\.write-probe" 2>nul
if not exist "%TMP%\.write-probe" (
  echo [ERROR] build temp dir "%TMP%" is not writable - cl.exe will fail with D8050.
  exit /b 1
)
del "%TMP%\.write-probe" >nul 2>&1

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

rem Deliberately do NOT set RC here. RC is the toolchain variable holding the
rem rc.exe path; embed-resource honours it VERBATIM and - once it is set - does
rem not fall back to its own discovery. Pinning it also gave our scripts a
rem mutable name to collide with: build.bat used RC for its exit code, so after
rem `build.bat debug` the release pass handed rc.exe = "0" and died with
rem "Are you sure you have RC.EXE in your $PATH or ${RC_$TARGET} or $RC is set?"
rem (2026-10-02). The two upstream mechanisms are enough and neither can be
rem corrupted by us:
rem   1) vcvars64 below puts <SDK>\bin\<ver>\x64 on PATH, so rc.exe resolves
rem   2) embed-resource discovers the Windows Kits via the registry / vswhere
rem Verified by deleting this probe and forcing the build script to re-run:
rem the resource step still found rc.exe. Keep this file ASCII-only (cmd
rem mis-parses multibyte comments and there is no chcp guard of its own).

rem C-dependency builds (cc-rs, e.g. vswhom-sys) need INCLUDE/LIB. cc-rs's own
rem MSVC detection misses the VS "18" layout on this machine, so seed the full
rem toolchain env via vcvars64 (located through vswhere; skipped when the
rem terminal already has one, e.g. a developer prompt).
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not defined INCLUDE if exist "%VSWHERE%" for /f "delims=" %%p in ('"%VSWHERE%" -latest -property installationPath 2^>nul') do (
  if exist "%%p\VC\Auxiliary\Build\vcvars64.bat" set "VCVARS=%%p\VC\Auxiliary\Build\vcvars64.bat"
)
if not defined INCLUDE if defined VCVARS call "%VCVARS%" >nul 2>&1
rem vcvars64 failing (half-uninstalled VS / broken license) must stop here:
rem continuing without INCLUDE/LIB only dies later inside cc-rs with errors
rem that look like "the toolchain is broken" - exactly what this script exists
rem to prevent.
if not defined INCLUDE if defined VCVARS (
  echo [ERROR] vcvars64 failed - set INCLUDE/LIB manually or repair Visual Studio
  exit /b 1
)
