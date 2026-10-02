@echo off
rem This file is pure ASCII BY DESIGN. History, twice over:
rem  1) chcp 65001 here broke subprocess-output parsing on CJK-locale Windows:
rem     cc-rs / embed_resource shell out to vswhere and mis-decode its GBK
rem     output under UTF-8 codepage, so cl/rc lost INCLUDE and the release
rem     build died with "RC.EXE not set" (fixed 2026-10-02).
rem  2) Multibyte rem comments mis-parse in some terminals (PowerShell) and
rem     execute comment fragments as commands. Keep every byte < 0x80.
rem All echoes are ASCII too - do not add non-ASCII content.
setlocal
rem Shared setup (ROOT/SRC/BIN, cargo PATH fallback, retry env) lives in common.bat.
rem The gitconfig-http.proxy warning there also explains the retry params.
call "%~dp0common.bat"
if errorlevel 1 exit /b 1

if not exist "%BIN%" mkdir "%BIN%"

rem Detect rustc host triple; cargo outputs exe to target\<triple>\<flavor>
set "TRIPLE="
for /f "tokens=2" %%i in ('rustc -vV 2^>nul ^| findstr /C:"host:"') do set "TRIPLE=%%i"
if "%TRIPLE%"=="" (
  echo   note: rustc probe failed, assuming x86_64-pc-windows-msvc
  set "TRIPLE=x86_64-pc-windows-msvc"
)

rem Usage: build.bat [debug^|release^|all^|package^|test]
set "FLAVOR=%~1"

rem Read version from Cargo.toml (package section, first unindented "version =")
set "RAWVER="
for /f "usebackq tokens=2 delims==" %%a in (`findstr /b /c:"version" "%SRC%\Cargo.toml"`) do (
  if not defined RAWVER set "RAWVER=%%a"
)
set "APPVER=%RAWVER:"=%"
set "APPVER=%APPVER: =%"
if "%APPVER%"=="" set "APPVER=0.0.0"
if "%FLAVOR%"=="" set "FLAVOR=all"

if not "%FLAVOR%"=="debug" if not "%FLAVOR%"=="release" if not "%FLAVOR%"=="all" if not "%FLAVOR%"=="package" if not "%FLAVOR%"=="test" (
  echo [ERROR] unknown flavor "%FLAVOR%" - use: debug, release, all, package or test
  exit /b 1
)

if "%FLAVOR%"=="debug" call :do_build debug
if "%FLAVOR%"=="release" call :do_build release
if "%FLAVOR%"=="all" (
  call :do_build debug
  if errorlevel 1 goto :fail
  call :do_build release
)
if "%FLAVOR%"=="test" (
  call :do_test
  if errorlevel 1 goto :fail
  goto :test_done
)

if "%FLAVOR%"=="package" call :do_package

if errorlevel 1 goto :fail
goto :done

:do_build
set "F=%~1"
echo.
echo =========================================
echo   Building %F% ...
echo =========================================
pushd "%SRC%"
if "%F%"=="release" (%CARGO_BIN% build --release) else (%CARGO_BIN% build)
set "RC=%errorlevel%"
popd
if %RC% neq 0 (
  echo Build failed for %F% with code %RC%
  exit /b 1
)
set "SRCDIR=%SRC%\target\%TRIPLE%\%F%"
if not exist "%SRCDIR%\niuma-timer.exe" set "SRCDIR=%SRC%\target\%F%"
if not exist "%BIN%\%F%" mkdir "%BIN%\%F%"
copy /Y "%SRCDIR%\niuma-timer.exe" "%BIN%\%F%\"
if errorlevel 1 (
  echo [ERROR] copy failed, exe not found at %SRCDIR%
  exit /b 1
)
rem PDB must ship beside its exe: a crash dump only symbolises against the
rem PDB of the same build (PE GUID+Age); an exe released without its PDB
rem means crashes of that build can never be symbolised.
if exist "%SRCDIR%\niuma_timer.pdb" copy /Y "%SRCDIR%\niuma_timer.pdb" "%BIN%\%F%\" >nul
echo Done: %BIN%\%F%\niuma-timer.exe
goto :eof

:do_package
rem CLI expects PRIVATE_KEY (a key path or contents); accept the newer PATH alias too.
if not defined TAURI_SIGNING_PRIVATE_KEY if defined TAURI_SIGNING_PRIVATE_KEY_PATH set "TAURI_SIGNING_PRIVATE_KEY=%TAURI_SIGNING_PRIVATE_KEY_PATH%"
echo.
echo =========================================
echo   Packaging (NSIS + MSI) ...
echo =========================================
pushd "%SRC%"
%CARGO_BIN% tauri build
set "RC=%errorlevel%"
popd
if %RC% neq 0 (
  echo [ERROR] Package failed with code %RC%
  echo Install tauri-cli first:  cargo install tauri-cli
  echo NSIS will be downloaded automatically on first package run
  exit /b 1
)
set "BUNDLE=%SRC%\target\%TRIPLE%\release\bundle"
if not exist "%BUNDLE%" set "BUNDLE=%SRC%\target\release\bundle"
rem Copy only artifacts matching the current version: cargo tauri build never
rem deletes stale bundles from older releases, so a bare *.exe/*.msi glob would
rem sweep old-version installers into bin\package (and onto the GitHub Release).
if not exist "%BIN%\package" mkdir "%BIN%\package"
copy /Y "%BUNDLE%\nsis\*%APPVER%*.exe" "%BIN%\package\"
copy /Y "%BUNDLE%\msi\*%APPVER%*.msi" "%BIN%\package\"
set "PORTABLE=%SRC%\target\%TRIPLE%\release\niuma-timer.exe"
if not exist "%PORTABLE%" set "PORTABLE=%SRC%\target\release\niuma-timer.exe"
copy /Y "%PORTABLE%" "%BIN%\package\niuma-timer-%APPVER%-portable.exe"
rem Portable-exe debug symbols ship with the release: take the PDB next to
rem the exe just copied and rename it with the version for asset identification.
rem Raw PDB is ~150 MB and proxy uploads to GitHub get reset (Errno 10054),
rem so compress it to .zip; tar ships with Windows 10/11 - no extra dependency.
rem tar is pinned to System32 bsdtar: -a with .zip needs bsdtar, and GNU
rem tar treats C: as a remote host - Git Bash PATH resolves GNU tar first.
set "PDBDIR=%PORTABLE%"
for %%p in ("%PORTABLE%") do set "PDBDIR=%%~dpp"
if exist "%PDBDIR%niuma_timer.pdb" (
  "%SystemRoot%\System32\tar.exe" -a -c -f "%BIN%\package\niuma-timer-%APPVER%-portable.pdb.zip" -C "%PDBDIR%." niuma_timer.pdb
  if errorlevel 1 (
    echo [ERROR] failed to compress niuma_timer.pdb for the release
    exit /b 1
  )
)
rem Updater signature files (.sig) must share the installer's name and be
rem copied into package together, or publish_release.py cannot build
rem latest.json and the whole auto-update chain dies.
if exist "%BUNDLE%\nsis\*%APPVER%*.exe.sig" copy /Y "%BUNDLE%\nsis\*%APPVER%*.exe.sig" "%BIN%\package\"
if exist "%BUNDLE%\msi\*%APPVER%*.msi.sig" copy /Y "%BUNDLE%\msi\*%APPVER%*.msi.sig" "%BIN%\package\"
rem The portable build is a hand-copied bare exe: if tauri produced a
rem signature for it, rename to the portable name so the .sig file carries
rem the version (release verification relies on it).
set "RAWSIG=%SRC%\target\%TRIPLE%\release\niuma-timer.exe.sig"
if not exist "%RAWSIG%" set "RAWSIG=%SRC%\target\release\niuma-timer.exe.sig"
if exist "%RAWSIG%" copy /Y "%RAWSIG%" "%BIN%\package\niuma-timer-%APPVER%-portable.exe.sig"
echo Done: %BIN%\package\
goto :eof

:do_test
echo.
echo =========================================
echo   Testing (cargo test + frontend assert scripts + release engine) ...
echo =========================================
pushd "%SRC%"
%CARGO_BIN% test --quiet
set "RC=%errorlevel%"
popd
if %RC% neq 0 (
  echo [ERROR] cargo test failed with code %RC%
  exit /b 1
)
where node >nul 2>&1
if errorlevel 1 (
  echo [ERROR] node not found - frontend test scripts in scripts\test_*.js need Node.js
  exit /b 1
)
node "%ROOT%scripts\run_all.js"
if errorlevel 1 exit /b 1
rem Release-engine regression tests (offline; guards the CI publishing path).
where python >nul 2>&1
if errorlevel 1 (
  echo [ERROR] python not found - scripts\test_publish_release.py needs Python 3
  exit /b 1
)
python "%ROOT%scripts\test_publish_release.py"
if errorlevel 1 exit /b 1
goto :eof

:test_done
echo.
echo All tests passed.
endlocal
exit /b 0

:fail
echo Build failed.
echo.
rem A failure whose text mentions RC.EXE / D8050 / a tool that could not be
rem started is almost always the toolchain process being denied at spawn, not a
rem source error. Print what the build was actually handed.
echo   Build-environment diagnostics:
echo     RC  = %RC%
echo     TMP = %TMP%
echo   Known machine-state issue on this box (see CHANGELOG): a freshly spawned
echo   toolchain process is occasionally denied under high parallelism. Retry, or
echo   serialise with:  cargo build --release -j1
exit /b 1

:done
echo.
echo Artifacts:
echo   %BIN%\debug\niuma-timer.exe
echo   %BIN%\release\niuma-timer.exe
echo   %BIN%\package\    (build.bat package: NSIS installer / MSI / portable exe)
endlocal
