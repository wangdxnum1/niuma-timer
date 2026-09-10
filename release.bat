@echo off
chcp 65001 >nul
setlocal EnableDelayedExpansion
rem ============================================================
rem  Niuma Timer - one-click build / package / release
rem
rem  Usage:
rem    release.bat              use current version from Cargo.toml
rem    release.bat 1.1.0        bump to 1.1.0 (syncs Cargo.toml + tauri.conf.json)
rem    release.bat 1.1.0 /y     no prompts (fully automatic)
rem ============================================================

set "ROOT=%~dp0"
set "SRC=%ROOT%src-tauri"
set "BIN=%ROOT%bin"
set "REPO=wangdxnum1/niuma-timer"

rem Cargo mirrors occasionally return 504; retry instead of failing outright.
set "CARGO_NET_RETRY=10"
set "CARGO_HTTP_TIMEOUT=180"

set "AUTO=no"
set "WANTVER=%~1"
if /i "%~1"=="/y" ( set "AUTO=yes" & set "WANTVER=" )
if /i "%~2"=="/y" set "AUTO=yes"

rem --- Prefer system Git: WorkBuddy's bundled PortableGit has a broken
rem     credential manager (segfaults). System Git + wincred works. ---
set "GIT=C:\Program Files\Git\cmd\git.exe"
if not exist "%GIT%" set "GIT=git"

echo.
echo =========================================
echo   Niuma Timer  Build / Package / Release
echo =========================================

rem ---------------- 1. version ----------------
set "RAWVER="
for /f "usebackq tokens=2 delims==" %%a in (`findstr /b /c:"version" "%SRC%\Cargo.toml"`) do (
  if not defined RAWVER set "RAWVER=%%a"
)
set "CURVER=%RAWVER:"=%"
set "CURVER=%CURVER: =%"
if "%CURVER%"=="" (
  echo [ERROR] cannot read version from Cargo.toml
  exit /b 1
)
if "%WANTVER%"=="" ( set "VER=%CURVER%" ) else ( set "VER=%WANTVER%" )
echo   current version : %CURVER%
echo   target  version : %VER%

if not "%VER%"=="%CURVER%" (
  echo.
  echo   syncing version %CURVER% to %VER% ...
  powershell -NoProfile -Command "$f='%SRC%\Cargo.toml'; $t=[IO.File]::ReadAllText($f); $q=[char]34; $p='(?m)^version\s*=\s*'+$q+[regex]::Escape('%CURVER%')+$q; $r='version = '+$q+'%VER%'+$q; $t=[regex]::Replace($t,$p,$r); [IO.File]::WriteAllText($f,$t)"
  if errorlevel 1 ( echo [ERROR] failed to update Cargo.toml & exit /b 1 )
  powershell -NoProfile -Command "$f='%SRC%\tauri.conf.json'; $t=[IO.File]::ReadAllText($f); $q=[char]34; $p=$q+'version'+$q+':\s*'+$q+[regex]::Escape('%CURVER%')+$q; $r=$q+'version'+$q+': '+$q+'%VER%'+$q; $t=[regex]::Replace($t,$p,$r); [IO.File]::WriteAllText($f,$t)"
  if errorlevel 1 ( echo [ERROR] failed to update tauri.conf.json & exit /b 1 )
  echo     Cargo.toml / tauri.conf.json updated
  findstr /c:"%VER%" "%SRC%\Cargo.toml" >nul
  if errorlevel 1 (
    echo [ERROR] version sync failed for Cargo.toml. Restore with:
    echo     git checkout -- src-tauri\Cargo.toml src-tauri\tauri.conf.json
    exit /b 1
  )
  findstr /c:"%VER%" "%SRC%\tauri.conf.json" >nul
  if errorlevel 1 (
    echo [ERROR] version sync failed for tauri.conf.json. Restore with:
    echo     git checkout -- src-tauri\Cargo.toml src-tauri\tauri.conf.json
    exit /b 1
  )
)

rem ---------------- 2. environment ----------------
where cargo >nul 2>&1
if errorlevel 1 (
  echo [ERROR] cargo not found. Install Rust first.
  exit /b 1
)
"%GIT%" --version >nul 2>&1
if errorlevel 1 (
  echo [ERROR] git not found.
  exit /b 1
)
cargo tauri --version >nul 2>&1
if errorlevel 1 (
  echo   tauri-cli is required for packaging but is not installed.
  set "INST=no"
  if "%AUTO%"=="yes" (
    set "INST=yes"
  ) else (
    set /p "INST=Install now? (takes several minutes) [Y/N] "
  )
  if /i "!INST!"=="Y" set "INST=yes"
  if "!INST!"=="no" (
    echo   Aborted. Install it manually:  cargo install tauri-cli --version "2"
    exit /b 1
  )
  echo   Installing tauri-cli ...
  echo   if the download fails with a mirror 504, retry with:
  echo    set CARGO_SOURCE_CRATES_IO_REPLACE_WITH=tuna
  echo    set CARGO_SOURCE_TUNA_REGISTRY=sparse+https://mirrors.tuna.tsinghua.edu.cn/crates.io-index/
  cargo install tauri-cli --version "2"
  if errorlevel 1 ( echo [ERROR] tauri-cli install failed & exit /b 1 )
  cargo tauri --version >nul 2>&1
  if errorlevel 1 ( echo [ERROR] tauri-cli still unavailable after install & exit /b 1 )
)

rem ---------------- 3. confirm ----------------
echo.
echo   Steps:
echo     1. cargo build --release
echo     2. cargo tauri build   (NSIS installer + MSI + portable exe)
echo     3. git commit / tag v%VER%
echo     4. git push origin main  +  push tag
echo     5. create GitHub Release and upload artifacts
echo.
if "%AUTO%"=="no" (
  set "ANS="
  set /p "ANS=Continue? [Y/N] "
  if /i not "!ANS!"=="Y" ( echo Aborted. & exit /b 0 )
)

rem ---------------- 4. clean old artifacts ----------------
if exist "%BIN%\package" (
  echo.
  echo   clearing old artifacts in bin\package ...
  del /q "%BIN%\package\*.*" >nul 2>&1
)

rem ---------------- 5. build ----------------
echo.
echo =========================================
echo   [1/5] Building release exe ...
echo =========================================
call "%ROOT%build.bat" release
if errorlevel 1 ( echo [ERROR] build failed & exit /b 1 )

rem ---------------- 6. package ----------------
echo.
echo =========================================
echo   [2/5] Packaging (NSIS + MSI) ...
echo =========================================
call "%ROOT%build.bat" package
if errorlevel 1 ( echo [ERROR] package failed & exit /b 1 )

rem ---------------- 7. verify artifacts ----------------
if not exist "%BIN%\package\*.exe" if not exist "%BIN%\package\*.msi" (
  echo [ERROR] no artifacts found in bin\package
  exit /b 1
)
echo.
echo   Artifacts in bin\package:
for %%f in ("%BIN%\package\*.exe" "%BIN%\package\*.msi") do echo     %%~nxf

rem ---------------- 8. commit ----------------
echo.
echo =========================================
echo   [3/5] Committing ...
echo =========================================
pushd "%ROOT%"
"%GIT%" add -A
"%GIT%" diff --cached --quiet
if errorlevel 1 (
  "%GIT%" commit -q -m "release: v%VER%"
  if errorlevel 1 ( echo [ERROR] commit failed & popd & exit /b 1 )
  echo     committed
) else (
  echo     nothing to commit, working tree clean
)

rem ---------------- 9. tag ----------------
"%GIT%" rev-parse "v%VER%" >nul 2>&1
if not errorlevel 1 (
  echo.
  echo   [WARN] tag v%VER% already exists.
  if "%AUTO%"=="no" (
    set "ANS="
    set /p "ANS=Delete and recreate it? [Y/N] "
    if /i not "!ANS!"=="Y" ( echo Aborted. & popd & exit /b 1 )
  )
  "%GIT%" tag -d "v%VER%"
)
"%GIT%" tag -a "v%VER%" -m "v%VER%"
if errorlevel 1 ( echo [ERROR] tag failed & popd & exit /b 1 )
echo     tag v%VER% created

rem ---------------- 10. push ----------------
echo.
echo =========================================
echo   [4/5] Pushing to GitHub ...
echo =========================================
rem Detect Clash proxy on 7890; schannel breaks through proxy, so force openssl.
set "PROXYARG="
netstat -an | findstr /C:"127.0.0.1:7890" | findstr /C:"LISTENING" >nul
if not errorlevel 1 (
  set "PROXYARG=-c http.proxy=http://127.0.0.1:7890 -c https.proxy=http://127.0.0.1:7890"
  echo     proxy 127.0.0.1:7890 detected, pushing through it
) else (
  echo     no local proxy detected, pushing directly
)
set "GITCFG=-c credential.helper=wincred -c http.sslBackend=openssl %PROXYARG%"

"%GIT%" %GITCFG% push origin main
if errorlevel 1 (
  echo [ERROR] push failed. If it is a TLS error, make sure the proxy is on, or run:
  echo     "%GIT%" -c credential.helper=wincred -c http.sslBackend=openssl push origin main
  popd
  exit /b 1
)
"%GIT%" %GITCFG% push origin "v%VER%"
if errorlevel 1 ( echo [ERROR] tag push failed & popd & exit /b 1 )
echo     pushed main and tag v%VER%

rem ---------------- 11. GitHub Release ----------------
echo.
echo =========================================
echo   [5/5] Creating GitHub Release ...
echo =========================================
set "ASSETS="
for %%f in ("%BIN%\package\*.exe" "%BIN%\package\*.msi") do set "ASSETS=!ASSETS! "%%f""

where gh >nul 2>&1
if errorlevel 1 (
  echo   gh CLI not found - falling back to browser.
  echo   Please create the release manually and drag these files in:
  for %%f in ("%BIN%\package\*.exe" "%BIN%\package\*.msi") do echo     %%f
  start "" "https://github.com/%REPO%/releases/new?tag=v%VER%"
  popd
  goto :summary
)

gh auth status >nul 2>&1
if errorlevel 1 (
  echo   gh is installed but not logged in. Run:  gh auth login
  echo   Falling back to browser.
  start "" "https://github.com/%REPO%/releases/new?tag=v%VER%"
  popd
  goto :summary
)

gh release create "v%VER%" --repo "%REPO%" --title "v%VER%" --generate-notes --latest !ASSETS!
if errorlevel 1 (
  echo [ERROR] gh release create failed; create it manually:
  start "" "https://github.com/%REPO%/releases/new?tag=v%VER%"
  popd
  exit /b 1
)
echo     release v%VER% published
popd

:summary
echo.
echo =========================================
echo   Done.
echo   Version  : %VER%
echo   Artifacts: %BIN%\package
echo   Release  : https://github.com/%REPO%/releases/tag/v%VER%
echo =========================================
endlocal
