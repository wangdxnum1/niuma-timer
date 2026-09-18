@echo off
rem This file is UTF-8 encoded; chcp 65001 below switches the console to UTF-8 so CJK echoes render (same as release.bat)
chcp 65001 >nul
setlocal
rem Shared setup (ROOT/SRC/BIN, cargo PATH fallback, retry env) lives in common.bat.
rem The gitconfig-http.proxy warning there also explains the retry params.
call "%~dp0common.bat"

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
echo Done: %BIN%\%F%\niuma-timer.exe
goto :eof

:do_package
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
if not exist "%BIN%\package" mkdir "%BIN%\package"
copy /Y "%BUNDLE%\nsis\*.exe" "%BIN%\package\"
copy /Y "%BUNDLE%\msi\*.msi" "%BIN%\package\"
set "PORTABLE=%SRC%\target\%TRIPLE%\release\niuma-timer.exe"
if not exist "%PORTABLE%" set "PORTABLE=%SRC%\target\release\niuma-timer.exe"
copy /Y "%PORTABLE%" "%BIN%\package\niuma-timer-%APPVER%-portable.exe"
echo Done: %BIN%\package\
goto :eof

:do_test
echo.
echo =========================================
echo   Testing (cargo test + frontend assert scripts) ...
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
goto :eof

:test_done
echo.
echo All tests passed.
endlocal
exit /b 0

:fail
echo Build failed.
exit /b 1

:done
echo.
echo Artifacts:
echo   %BIN%\debug\niuma-timer.exe
echo   %BIN%\release\niuma-timer.exe
echo   %BIN%\package\    (build.bat package: NSIS installer / MSI / portable exe)
endlocal
