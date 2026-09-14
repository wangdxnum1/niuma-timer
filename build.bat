@echo off
rem 本文件为 UTF-8 编码：不切代码页的话，默认 GBK 控制台会把中文显示成乱码（release.bat 同款）
chcp 65001 >nul
setlocal
set "ROOT=%~dp0"
set "SRC=%ROOT%src-tauri"
set "BIN=%ROOT%bin"

if not exist "%BIN%" mkdir "%BIN%"

rem 镜像偶发抽风的兜底参数（与 release.bat 一致）。另注：cargo 会继承 ~/.gitconfig
rem 的全局 http.proxy——全局代理会把 cargo 的 TLS 握手掐断（Clash 的 SOCKS5 下必现），
rem 该代理已改为仅对 github.com 生效，cargo 直连 rsproxy 毫无压力，勿改回全局。
set "CARGO_NET_RETRY=10"
set "CARGO_HTTP_TIMEOUT=180"

rem cargo 兜底：rustup 安装时可能没把 .cargo\bin 写进 PATH（装完后新开的终端才看得到），
rem 这里自行探测补上；同时兼容 CARGO_HOME 自定义、rustup 已写 PATH 两种情况
set "CARGO_DIR="
where cargo >nul 2>&1
if not errorlevel 1 goto :cargo_ok
if exist "%USERPROFILE%\.cargo\bin\cargo.exe" set "CARGO_DIR=%USERPROFILE%\.cargo\bin"
if not defined CARGO_DIR if exist "%CARGO_HOME%\bin\cargo.exe" set "CARGO_DIR=%CARGO_HOME%\bin"
if not defined CARGO_DIR (
  for /f "delims=" %%p in ('rustup which cargo 2^>nul') do set "CARGO_DIR=%%~dpp"
)
if defined CARGO_DIR (
  set "PATH=%CARGO_DIR%;%PATH%"
  goto :cargo_ok
)
echo [ERROR] 未在 PATH 中找到 cargo，也未能在以下位置探测到：
echo   %USERPROFILE%\.cargo\bin\cargo.exe
echo   %CARGO_HOME%\bin\cargo.exe   (CARGO_HOME=%CARGO_HOME%)
echo   请先安装 Rust (https://rustup.rs)，或把 .cargo\bin 加入系统 PATH 后重开终端重试。
exit /b 1
:cargo_ok

rem 探测 rustc host triple，cargo 会按 target\<triple>\<flavor> 输出 exe
set "TRIPLE="
for /f "tokens=2" %%i in ('rustc -vV 2^>nul ^| findstr /C:"host:"') do set "TRIPLE=%%i"
if "%TRIPLE%"=="" set "TRIPLE=x86_64-pc-windows-msvc"

rem Usage: build.bat [debug|release|all|package]
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

if "%FLAVOR%"=="debug" call :do_build debug
if "%FLAVOR%"=="release" call :do_build release
if "%FLAVOR%"=="all" (
  call :do_build debug
  if errorlevel 1 goto :fail
  call :do_build release
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
if "%F%"=="release" (cargo build --release) else (cargo build)
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
cargo tauri build
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
