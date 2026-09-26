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
rem  Publishing needs no gh CLI: it reuses the GitHub token already
rem  stored by Git Credential Manager (scripts/publish_release.py).
rem ============================================================

rem Shared setup (ROOT/SRC/BIN, cargo PATH fallback, retry env) lives in common.bat
call "%~dp0common.bat"
set "REPO=wangdxnum1/niuma-timer"

set "AUTO=no"
set "WANTVER=%~1"
if /i "%~1"=="/y" ( set "AUTO=yes" & set "WANTVER=" )
if /i "%~2"=="/y" set "AUTO=yes"
if "%AUTO%"=="yes" set "CI=true"

rem --- Prefer system Git: WorkBuddy's bundled PortableGit has a broken
rem     credential manager (segfaults). System Git + wincred works. ---
set "GIT=C:\Program Files\Git\cmd\git.exe"
if not exist "%GIT%" set "GIT=git"

rem Push / tag / release all key off the current branch; the flow assumes main.
rem A non-main branch would make the tag point at a commit that is not on main.
set "BRANCH="
for /f "usebackq delims=" %%b in (`"%GIT%" branch --show-current 2^>nul`) do set "BRANCH=%%b"
if "%BRANCH%"=="" set "BRANCH=(unknown)"

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
  rem verify the sync actually landed: re-parse both files instead of just
  rem checking that the version string appears somewhere
  set "GOTVER="
  for /f "usebackq tokens=2 delims==" %%a in (`findstr /b /c:"version" "%SRC%\Cargo.toml"`) do (
    if not defined GOTVER set "GOTVER=%%a"
  )
  set "GOTVER=!GOTVER:"=!"
  set "GOTVER=!GOTVER: =!"
  if not "!GOTVER!"=="%VER%" (
    echo [ERROR] version sync failed for Cargo.toml. Restore with:
    echo     git checkout -- src-tauri\Cargo.toml src-tauri\tauri.conf.json
    exit /b 1
  )
  set "JSONVER="
  for /f "usebackq delims=" %%a in (`powershell -NoProfile -Command "(Get-Content -Raw -Encoding UTF8 '%SRC%\tauri.conf.json' | ConvertFrom-Json).version"`) do set "JSONVER=%%a"
  if not "!JSONVER!"=="%VER%" (
    echo [ERROR] version sync failed for tauri.conf.json. Restore with:
    echo     git checkout -- src-tauri\Cargo.toml src-tauri\tauri.conf.json
    exit /b 1
  )
)

rem ---------------- 2. environment ----------------
rem cargo PATH fallback is already handled in common.bat; this is the final check.
where cargo >nul 2>&1
if errorlevel 1 (
  echo [ERROR] cargo not found. Install Rust first.
  echo         If Rust was just installed, reopen the terminal window and retry.
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

rem 更新包签名：tauri 用私钥给安装包产出 .sig，客户端靠它校验下载到的包。
rem 缺了不会报错、只会静默产出一个没签名的包，到用户侧才发现更新被拒 —— 提前拦住。
if not defined TAURI_SIGNING_PRIVATE_KEY if defined TAURI_SIGNING_PRIVATE_KEY_PATH set "TAURI_SIGNING_PRIVATE_KEY=%TAURI_SIGNING_PRIVATE_KEY_PATH%"
if not defined TAURI_SIGNING_PRIVATE_KEY (
  echo [ERROR] TAURI_SIGNING_PRIVATE_KEY is not set - updater artifacts would be unsigned.
  echo         PowerShell: $env:TAURI_SIGNING_PRIVATE_KEY = "$HOME\.tauri\niuma-timer.key"
  exit /b 1
)
rem TAURI_SIGNING_PRIVATE_KEY_PASSWORD is optional for an unencrypted local key.

rem ---------------- 3. confirm ----------------
echo.
echo   Steps:
echo     0. tests (cargo test + frontend assert scripts)
echo     1. cargo build --release
echo     2. cargo tauri build   (NSIS installer + MSI + portable exe)
echo     3. git commit / tag v%VER%
echo     4. git push origin %BRANCH%  +  push tag
echo     5. create GitHub Release and upload artifacts
echo.
if not "%BRANCH%"=="main" (
  echo   [WARN] current branch is "%BRANCH%", not main - the tag would point
  echo          at a commit that is not on main.
  if "%AUTO%"=="yes" (
    echo [ERROR] refusing to release from a non-main branch in /y mode.
    exit /b 1
  )
  set "ANS="
  set /p "ANS=Continue releasing from %BRANCH%? [y/N] "
  if /i not "!ANS!"=="Y" ( echo Aborted. & exit /b 0 )
)
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

rem ---------------- 5. tests ----------------
echo.
echo =========================================
echo   [0/5] Testing (cargo test + frontend assert scripts) ...
echo =========================================
call "%ROOT%build.bat" test
if errorlevel 1 ( echo [ERROR] tests failed & exit /b 1 )

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
rem Every artifact must carry the target version in its name: a file without
rem it is a stale bundle from an older release that leaked into bin\package.
set "BADART="
echo.
echo   Artifacts in bin\package:
for %%f in ("%BIN%\package\*.exe" "%BIN%\package\*.msi") do (
  echo     %%~nxf
  set "NAME=%%~nxf"
  if "!NAME:%VER%=!"=="!NAME!" set "BADART=!BADART! %%~nxf"
)
if defined BADART (
  echo [ERROR] artifacts without version %VER% found in bin\package:!BADART!
  echo         Clean bin\package and src-tauri\target\...\bundle, then rerun.
  exit /b 1
)
rem 没有 .sig 就生成不出 latest.json，发出去的版本永远收不到更新
if not exist "%BIN%\package\*.exe.sig" (
  echo [ERROR] no .exe.sig in bin\package - updater artifacts were not signed.
  echo         Set TAURI_SIGNING_PRIVATE_KEY and TAURI_SIGNING_PRIVATE_KEY_PASSWORD, then repackage.
  exit /b 1
)

rem  Pick a Python interpreter for the gh-less publish path
set "PYEXE="
where py >nul 2>&1
if not errorlevel 1 set "PYEXE=py -3"
if not defined PYEXE (
  where python >nul 2>&1
  if not errorlevel 1 set "PYEXE=python"
)

rem Metadata is mandatory before git or network side effects.
if not defined PYEXE (
  echo [ERROR] Python 3 is required to validate release metadata.
  exit /b 1
)
(
  %PYEXE% "%ROOT%scripts\publish_release.py" --tag "v%VER%" --version "%VER%" --package "%BIN%\package" --repo "%REPO%" --generate-notes-only
  if errorlevel 1 exit /b 1
)
call :verify_latest
if errorlevel 1 exit /b 1


rem ---------------- 8. commit ----------------
echo.
echo =========================================
echo   [3/5] Committing ...
echo =========================================
pushd "%ROOT%"
rem Show what add -A is about to sweep in (also in /y mode) so nothing
rem unexpected - stray temp files, secrets - gets committed sight unseen.
echo   files about to be committed:
"%GIT%" status --porcelain
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

"%GIT%" %GITCFG% push origin "%BRANCH%"
if errorlevel 1 (
  echo [ERROR] push failed. If it is a TLS error, make sure the proxy is on, or run:
  echo     "%GIT%" -c credential.helper=wincred -c http.sslBackend=openssl push origin "%BRANCH%"
  popd
  exit /b 1
)
"%GIT%" %GITCFG% push origin "v%VER%"
if errorlevel 1 (
  echo [ERROR] tag push failed. If the remote tag v%VER% already exists from
  echo         a previous release of this version, force-update it with:
  echo     "%GIT%" push origin "v%VER%" --force
  popd
  exit /b 1
)
echo     pushed %BRANCH% and tag v%VER%

rem ---------------- 11. GitHub Release ----------------
echo.
echo =========================================
echo   [5/5] Creating GitHub Release ...
echo =========================================
set "ASSETS="
for %%f in ("%BIN%\package\*.exe" "%BIN%\package\*.msi") do set "ASSETS=!ASSETS! "%%f""
rem 签名与 updater 清单同样是 Release 资产：客户端按 latest.json 找安装包，
rem 少一个都会让自动更新 404
for %%f in ("%BIN%\package\*.exe.sig" "%BIN%\package\*.msi.sig") do set "ASSETS=!ASSETS! "%%f""
rem Manifest and checksums are generated above, so collect them after generation.
if exist "%BIN%\package\latest.json" set "ASSETS=!ASSETS! "%BIN%\package\latest.json""
if exist "%BIN%\package\SHA256SUMS.txt" set "ASSETS=!ASSETS! "%BIN%\package\SHA256SUMS.txt""

where gh >nul 2>&1
if not errorlevel 1 (
  gh auth status >nul 2>&1
  if not errorlevel 1 goto :publish_gh
)

if defined PYEXE goto :publish_py
goto :publish_browser

:publish_gh
echo   publishing with gh CLI ...
if exist "%BIN%\package\RELEASE_NOTES.md" (
  gh release create "v%VER%" --repo "%REPO%" --title "Niuma Timer %VER%" --notes-file "%BIN%\package\RELEASE_NOTES.md" --draft !ASSETS!
) else (
  gh release create "v%VER%" --repo "%REPO%" --title "Niuma Timer %VER%" --generate-notes --draft !ASSETS!
)
if errorlevel 1 (
  echo [ERROR] gh release create failed; falling back to browser.
  goto :publish_browser
)
gh release edit "v%VER%" --repo "%REPO%" --draft=false --latest
if errorlevel 1 goto :release_fail
echo     release v%VER% published
call :verify_latest
if errorlevel 1 goto :release_fail
popd
goto :summary

:publish_py
echo   gh not available - publishing via GitHub API with the stored git credential ...
%PYEXE% "%ROOT%scripts\publish_release.py" --tag "v%VER%" --version "%VER%" --package "%BIN%\package" --repo "%REPO%"
if errorlevel 1 (
  echo [ERROR] API publish failed; falling back to browser.
  goto :publish_browser
)
echo     release v%VER% published
call :verify_latest
if errorlevel 1 goto :release_fail
popd
goto :summary

:publish_browser
echo   Create the release manually and drag these files in:
for %%f in ("%BIN%\package\*.exe" "%BIN%\package\*.msi") do echo     %%f
if exist "%BIN%\package\latest.json" echo     %BIN%\package\latest.json
start "" "https://github.com/%REPO%/releases/new?tag=v%VER%"
popd
goto :summary

:verify_latest
echo   verifying latest.json ...
if not exist "%BIN%\package\latest.json" (
  echo [ERROR] latest.json missing in bin\package - automatic update would break.
  exit /b 1
)
powershell -NoProfile -Command "$j = Get-Content -Raw -Encoding UTF8 '%BIN%\package\latest.json' | ConvertFrom-Json; if (@($j.platforms.PSObject.Properties).Count -lt 1) { exit 1 }"
if errorlevel 1 (
  echo [ERROR] latest.json has an empty platforms map - no client could update.
  exit /b 1
)
echo     latest.json ok
exit /b 0

:release_fail
echo [ERROR] release v%VER% was published but latest.json verification failed.
echo         Fix bin\package and rerun scripts\publish_release.py for this tag.
endlocal
exit /b 1

:summary
echo.
echo =========================================
echo   Done.
echo   Version  : %VER%
echo   Artifacts: %BIN%\package
echo   Release  : https://github.com/%REPO%/releases/tag/v%VER%
echo =========================================
endlocal
