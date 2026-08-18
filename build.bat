@echo off
setlocal EnableDelayedExpansion
set "ROOT=%~dp0"
set "SRC=%ROOT%src-tauri"
set "BIN=%ROOT%bin"

set "FLAVOR=%1"
if "%FLAVOR%"=="" set "FLAVOR=all"

if not exist "%BIN%" mkdir "%BIN%"

if "%FLAVOR%"=="debug"   call :build debug   & if errorlevel 1 goto :fail
if "%FLAVOR%"=="release" call :build release & if errorlevel 1 goto :fail
if "%FLAVOR%"=="all" (
    call :build debug   & if errorlevel 1 goto :fail
    call :build release & if errorlevel 1 goto :fail
)
goto :done

:build
set "F=%1"
echo.
echo =========================================
echo   Building %F% ...
echo =========================================
pushd "%SRC%"
if "%F%"=="release" ( cargo build --release ) else ( cargo build )
set "RC=%errorlevel%"
popd
if %RC% neq 0 (
    echo [ERROR] cargo build (%F%) failed with code %RC%
    exit /b 1
)
if "%F%"=="release" (
    if not exist "%BIN%\release" mkdir "%BIN%\release"
    copy /Y "%SRC%\target\release\niuma-timer.exe" "%BIN%\release\"
) else (
    if not exist "%BIN%\debug" mkdir "%BIN%\debug"
    rem copy exe into bin\debug (keep target name)
    copy /Y "%SRC%\target\debug\niuma-timer.exe" "%BIN%\debug\"
)
echo [OK] niuma-timer.exe -^> %BIN%\%F%\
goto :eof

:fail
echo Build failed.
exit /b 1

:done
echo.
echo All done. Artifacts:
echo   %BIN%\debug\niuma-timer.exe
echo   %BIN%\release\niuma-timer.exe
endlocal
