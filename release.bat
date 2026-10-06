@echo off
rem Pure ASCII BY DESIGN, and no chcp here - see the header of build.bat:
rem a UTF-8 code page makes child tools (vswhere via cc-rs / embed_resource)
rem mis-decode GBK output, which cost us "RC.EXE not set" on 2026-10-02.
setlocal EnableDelayedExpansion
rem ============================================================
rem  Niuma Timer - one-click build / package / tag / push
rem
rem  Usage:
rem    release.bat              use current version from Cargo.toml
rem    release.bat 1.1.0        bump to 1.1.0 (syncs Cargo.toml + tauri.conf.json)
rem    release.bat 1.1.0 /y     no prompts (fully automatic)
rem
rem  Publishing lives in CI (phase 2): pushing tag vX.Y.Z triggers
rem  .github/workflows/release.yml, which rebuilds, packages, and
rem  creates the GitHub Release. Watch progress at:
rem    https://github.com/wangdxnum1/niuma-timer/actions
rem  Emergency manual publish (reuses Git credential manager):
rem    python scripts\publish_release.py --tag vX.Y.Z --version X.Y.Z --package bin\package
rem ============================================================

rem Shared setup (ROOT/SRC/BIN, cargo PATH fallback, retry env) lives in common.bat
call "%~dp0common.bat"
if errorlevel 1 exit /b 1
rem --- Single source of repo slug: keep in sync with scripts/push_via_api.py
rem     REPO (scripts/test_local_gate.js asserts both literals match). ---
set "REPO=wangdxnum1/niuma-timer"

set "AUTO=no"
set "WANTVER=%~1"
if /i "%~1"=="/y" ( set "AUTO=yes" & set "WANTVER=" )
if /i "%~2"=="/y" set "AUTO=yes"
if "%AUTO%"=="yes" set "CI=true"

rem --- Prefer system Git: WorkBuddy's bundled PortableGit has a broken
rem     credential manager (segfaults). System Git + wincred works. Fallback
rem     probes PATH but refuses PortableGit/WorkBuddy copies (fail-closed). ---
set "GIT=C:\Program Files\Git\cmd\git.exe"
if not exist "%GIT%" (
    set "GIT="
    for /f "delims=" %%G in ('where git 2^>nul') do (
        if not defined GIT (
            echo %%G | findstr /i "PortableGit WorkBuddy" >nul
            if errorlevel 1 set "GIT=%%G"
        )
    )
)
if not defined GIT (
    echo [ERROR] No usable git found. WorkBuddy PortableGit is rejected because
    echo         its credential helper segfaults. Install system Git first.
    exit /b 1
)

rem Push / tag / release all key off the current branch; the flow assumes main.
rem A non-main branch would make the tag point at a commit that is not on main.
set "BRANCH="
pushd "%ROOT%"
if errorlevel 1 ( echo [ERROR] cannot enter repository root & exit /b 1 )
for /f "usebackq delims=" %%b in (`"%GIT%" branch --show-current 2^>nul`) do set "BRANCH=%%b"
popd
if "%BRANCH%"=="" set "BRANCH=(unknown)"

echo.
echo =========================================
echo   Niuma Timer  Build / Package / Release
echo =========================================

if not "%BRANCH%"=="main" (
  echo [ERROR] current branch is "%BRANCH%", not main.
  echo         Cloud publishing accepts only tags whose commits are on main.
  exit /b 1
)

rem ---------------- 1. version ----------------
rem Python is needed here already: the version bump must cover ALL five places
rem (Cargo.toml / tauri.conf.json / README x2 / CHANGELOG rename) or the
rem test_readme gate dies mid-release (two manual misses 2026-10-05/06).
set "PYEXE="
where py >nul 2>&1
if not errorlevel 1 set "PYEXE=py -3"
if not defined PYEXE (
  where python >nul 2>&1
  if not errorlevel 1 set "PYEXE=python"
)
if not defined PYEXE (
  echo [ERROR] Python 3 is required for release metadata and docs sync.
  exit /b 1
)
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
  rem Docs move with the version too: README x2 + CHANGELOG section rename.
  rem Without this the test_readme gate dies mid-release and leaves a dirty tree.
  %PYEXE% "%ROOT%scripts\sync_release_docs.py" "%VER%"
  if errorlevel 1 (
    echo [ERROR] release docs sync failed - prepare CHANGELOG entries and retry.
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

rem Updater signing: tauri signs the installers with the private key, producing
rem .sig files the client uses to verify what it downloaded.
rem A missing key does NOT fail the build - it silently ships unsigned
rem packages and users only discover the rejected update later. Stop it here.
rem Key resolution order: env TAURI_SIGNING_PRIVATE_KEY -> env
rem TAURI_SIGNING_PRIVATE_KEY_PATH -> the conventional per-user key at
rem %USERPROFILE%\.tauri\niuma-timer.key. The default keeps "release.bat X /y"
rem a true zero-prep one command; only an unknown key location must be set
rem by hand (2026-10-03 release: an env var lost to shell quoting made the
rem one-click flow fail at this gate - the default removes that failure mode).
if not defined TAURI_SIGNING_PRIVATE_KEY if defined TAURI_SIGNING_PRIVATE_KEY_PATH set "TAURI_SIGNING_PRIVATE_KEY=%TAURI_SIGNING_PRIVATE_KEY_PATH%"
if not defined TAURI_SIGNING_PRIVATE_KEY if exist "%USERPROFILE%\.tauri\niuma-timer.key" set "TAURI_SIGNING_PRIVATE_KEY=%USERPROFILE%\.tauri\niuma-timer.key"
if not defined TAURI_SIGNING_PRIVATE_KEY (
  echo [ERROR] TAURI_SIGNING_PRIVATE_KEY is not set - updater artifacts would be unsigned.
  echo         Default key not found at %%USERPROFILE%%\.tauri\niuma-timer.key either.
  echo         Put the key there, or set it now:
  echo         PowerShell: $env:TAURI_SIGNING_PRIVATE_KEY = "$HOME\.tauri\niuma-timer.key"
  exit /b 1
)
rem TAURI_SIGNING_PRIVATE_KEY_PASSWORD is optional for an unencrypted local key.

rem ---------------- 3. confirm ----------------
echo.
echo   Steps:
echo     0. tests (cargo test + frontend assert scripts + release engine)
echo     1. cargo build --release
echo     2. cargo tauri build   (NSIS installer + MSI + portable exe)
echo     3. git commit / tag v%VER%
echo     4. git push origin %BRANCH%  +  push tag
echo     5. GitHub Actions builds and publishes the Release (watch the Actions page)
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

rem ---------------- 5. tests ----------------
echo.
echo =========================================
echo   [0/5] Testing (cargo test + frontend assert scripts + release engine) ...
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
rem Without a .sig there is no latest.json, so the shipped version
rem can never receive an update.
if not exist "%BIN%\package\*.exe.sig" (
  echo [ERROR] no .exe.sig in bin\package - updater artifacts were not signed.
  echo         Set TAURI_SIGNING_PRIVATE_KEY and TAURI_SIGNING_PRIVATE_KEY_PASSWORD, then repackage.
  exit /b 1
)
rem Without a PDB no crash dump from the field can ever be symbolised:
rem shipping this version means giving up on diagnosing it.
rem PDBs ship as .pdb.zip; a raw .pdb is accepted too, so a manual
rem upload can still repair a release.
if not exist "%BIN%\package\*.pdb.zip" if not exist "%BIN%\package\*.pdb" (
  echo [ERROR] no .pdb in bin\package - crash dumps for this release could never be symbolised.
  echo         Check that target\...\release\niuma_timer.pdb exists, then repackage.
  exit /b 1
)

rem Metadata is mandatory before git or network side effects.
rem (PYEXE probed in step 1 - the docs sync needs it before the version bump.)
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
"%GIT%" show-ref --verify --quiet "refs/tags/v%VER%"
if not errorlevel 1 (
  set "HEAD_COMMIT="
  set "TAG_COMMIT="
  for /f "delims=" %%h in ('"%GIT%" rev-parse HEAD 2^>nul') do set "HEAD_COMMIT=%%h"
  for /f "delims=" %%h in ('"%GIT%" rev-list -n 1 refs/tags/v%VER% 2^>nul') do set "TAG_COMMIT=%%h"
  if not defined TAG_COMMIT (
    echo [ERROR] cannot resolve existing tag v%VER% to a commit. Refusing to replace it.
    popd
    exit /b 1
  )
  if not "!TAG_COMMIT!"=="!HEAD_COMMIT!" (
    echo [ERROR] tag v%VER% points to !TAG_COMMIT!, but HEAD is !HEAD_COMMIT!.
    echo         Release tags are immutable. Use a new version after correcting the commit.
    popd
    exit /b 1
  )
  echo     tag v%VER% already points to HEAD; reusing it
) else (
  "%GIT%" tag -a "v%VER%" -m "v%VER%"
  if errorlevel 1 ( echo [ERROR] tag failed & popd & exit /b 1 )
  echo     tag v%VER% created
)

rem ---------------- 10. push ----------------
echo.
echo =========================================
echo   [4/5] Pushing to GitHub ...
echo =========================================
rem Detect Clash proxy on 7890; schannel breaks through proxy, so force openssl.
rem When the probe concludes "direct", the http/https proxy must be cleared
rem explicitly: a global gitconfig may still carry a leftover
rem socks5://127.0.0.1:7890, and with the proxy app closed it silently takes
rem over the connection, so the push fails with
rem "Failed to connect over proxy 127.0.0.1" - contradicting the probe itself.
set "PROXYARG="
netstat -an | findstr /C:"127.0.0.1:7890" | findstr /C:"LISTENING" >nul
if not errorlevel 1 (
  set "PROXYARG=-c http.proxy=http://127.0.0.1:7890 -c https.proxy=http://127.0.0.1:7890"
  echo     proxy 127.0.0.1:7890 detected, pushing through it
) else (
  set "PROXYARG=-c http.proxy= -c https.proxy="
  echo     no local proxy detected, pushing directly (overriding global git proxy config)
)
set "GITCFG=-c credential.helper=wincred -c http.sslBackend=openssl %PROXYARG%"

rem Branch push retries: transient connection resets on the flaky github.com
rem route die per-connection, so a retry usually lands (v1.9.0 tag needed 3
rem tries). Falling straight to the Git Data API rebuilds the remote branch
rem with NEW SHAs and forces a manual fetch+reset alignment - keep that as the
rem last resort for a truly dead route, not for a transient reset.
set "BRTRY=0"
:br_retry
set /a BRTRY+=1
"%GIT%" %GITCFG% push origin "%BRANCH%"
if not errorlevel 1 goto :br_done
if %BRTRY% GEQ 5 goto :br_api_fallback
echo     branch push attempt %BRTRY% of 5 failed - retrying in 3 seconds ...
ping -n 4 127.0.0.1 >nul
goto :br_retry

:br_api_fallback
echo     branch push failed 5 times - rebuilding %BRANCH% via the Git Data API.
echo     Remote commits get new SHAs. Align local afterwards with
echo     git fetch origin + git reset --hard origin/%BRANCH% - see scripts\push_via_api.py.
where python >nul 2>&1
if errorlevel 1 (
  echo [ERROR] python not found - API fallback needs it
  popd
  exit /b 1
)
rem Branch AND tag in one API call: the API-rebuilt remote head has a
rem different sha than the local one, so pushing the local tag with git
rem here would point it at a nonexistent commit and trigger CI on it.
python "%ROOT%scripts\push_via_api.py" "%BRANCH%" --base "origin/%BRANCH%" --tag "v%VER%"
if errorlevel 1 (
  echo [ERROR] API fallback push failed
  popd
  exit /b 1
)
echo     pushed %BRANCH% and tag v%VER% via Git Data API
set "API_PUSHED=yes"
goto :done

:br_done
rem Tag push retries + API-only fallback. The branch push above opens its own
rem connection; on a flaky github.com route the tag push can die alone with
rem "Connection was reset" while the branch landed (v1.9.0, 2026-10-05). A failed
rem tag push never created the tag on the remote, so retrying cannot force-update
rem anything. The fallback is safe for the same reason: the branch push already
rem succeeded, so the remote head equals the local sha; push_via_api.py creates
rem the tag only after verifying the remote head matches the local tag target.
set "TAGTRY=0"
:tag_retry
set /a TAGTRY+=1
"%GIT%" %GITCFG% push origin "v%VER%"
if not errorlevel 1 goto :tag_pushed
if %TAGTRY% GEQ 5 goto :tag_api_fallback
rem ping as a portable sleep: timeout fails outright when stdin is redirected
ping -n 4 127.0.0.1 >nul
goto :tag_retry

:tag_api_fallback
echo     tag push failed 5 times - creating tag v%VER% via the Git Data API.
echo     Safe: the branch push already succeeded, so the remote head equals the
echo     local sha, and the tool refuses any head-vs-tag-target mismatch.
where python >nul 2>&1
if errorlevel 1 (
  echo [ERROR] python not found - API tag fallback needs it
  popd
  exit /b 1
)
python "%ROOT%scripts\push_via_api.py" "%BRANCH%" --base "origin/%BRANCH%" --tag "v%VER%"
if errorlevel 1 (
  echo [ERROR] API tag fallback failed. If refs/tags/v%VER% already exists at
  echo         another commit it is immutable - use a new version number.
  popd
  exit /b 1
)
echo     tag v%VER% created on the remote via the Git Data API

:tag_pushed
echo     pushed %BRANCH% and tag v%VER%
goto :done

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

:done
rem ---------------- 11. done - the Release itself is built & published by CI ----------------
rem Publishing now runs in the cloud: the tag triggers
rem .github/workflows/release.yml, which rebuilds and creates the GitHub Release.
rem Emergency path (CI unavailable and the release cannot wait - run the
rem publishing engine locally, using the token stored by Git Credential Manager):
rem   python scripts\publish_release.py --tag v%VER% --version %VER% --package bin\package
echo.
echo =========================================
echo   Done locally. Branch %BRANCH% and tag v%VER% pushed.
echo   GitHub Actions will now rebuild, package and publish:
echo     https://github.com/%REPO%/actions
echo   api.github.com is reachable even when github.com is not - watch the run:
echo     https://api.github.com/repos/%REPO%/actions/runs?per_page=1
echo   Pre-flight artifacts kept in: %BIN%\package
if defined API_PUSHED (
  echo   NOTE: the remote branch was rebuilt via the API with NEW SHAs.
  echo   Align local before committing anything else, or the next push diverges:
  echo     git fetch origin ^&^& git reset --hard origin/%BRANCH%
)
echo =========================================
popd
endlocal
