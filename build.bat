@echo off
setlocal
set "ROOT=%~dp0"
set "SRC=%ROOT%src-tauri"
set "BIN=%ROOT%bin"

if not exist "%BIN%" mkdir "%BIN%"

rem 探测 rustc host triple，cargo 会按 target\<triple>\<flavor> 输出 exe
set "TRIPLE="
for /f "tokens=2" %%i in ('rustc -vV 2^>nul ^| findstr /C:"host:"') do set "TRIPLE=%%i"
if "%TRIPLE%"=="" set "TRIPLE=x86_64-pc-windows-msvc"

set "FLAVOR=%~1"
if "%FLAVOR%"=="" set "FLAVOR=all"

if "%FLAVOR%"=="debug"   call :do_build debug
if "%FLAVOR%"=="release" call :do_build release
if "%FLAVOR%"=="all" (
    call :do_build debug
    if errorlevel 1 goto :fail
    call :do_build release
)

if errorlevel 1 goto :fail
goto :done

:do_build
set "F=%~1"
echo.
echo =========================================
echo   Building %F% ...
echo =========================================
pushd "%SRC%"
if "%F%"=="release" (
    cargo build --release
) else (
    cargo build
)
set "RC=%errorlevel%"
popd
if %RC% neq 0 (
    echo Build failed for %F% with code %RC%
    exit /b 1
)

rem 实际 exe 位置：优先 target/<triple>/<flavor>，兜底 target/<flavor>
set "SRCDIR=%SRC%\target\%TRIPLE%\%F%"
if not exist "%SRCDIR%\niuma-timer.exe" set "SRCDIR=%SRC%\target\%F%"
if not exist "%BIN%\%F%" mkdir "%BIN%\%F%"
copy /Y "%SRCDIR%\niuma-timer.exe" "%BIN%\%F%\"
if errorlevel 1 (
    echo [ERROR] copy failed: %SRCDIR%\niuma-timer.exe not found
    exit /b 1
)
echo Done: %BIN%\%F%\niuma-timer.exe
goto :eof

:fail
echo Build failed.
exit /b 1

:done
echo.
echo Artifacts:
echo   %BIN%\debug\niuma-timer.exe
echo   %BIN%\release\niuma-timer.exe
endlocal
