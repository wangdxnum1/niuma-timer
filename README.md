# NiuMa Timer

> Track how much you've earned today, down to the second — plus what you actually did all day.
> [Chinese docs](./README.zh-CN.md)
> Current version **1.9.0** (2026-10-05) · [Changelog](./CHANGELOG.md)

A lightweight Windows system-tray tool for wage workers. It sits in your tray and shows, in real time:

- How much you've earned **today** (¥)
- How long you've been **working** (hours/minutes/seconds)
- How long **until you're off work**
- Your **money rate** (¥/min) and **days until payday**

The tray icon itself renders the earned amount — just glance at the taskbar to feel the ¥/sec.

Beyond wage tracking, it doubles as a **desktop-behavior dashboard**: overtime, mouse/keyboard activity, per-app usage time, and media playback time are all tracked locally (SQLite) and viewable from the tray.

## Features

- **Today-only accounting** — resets at midnight, no carry-over
- **Tray icon draws the amount** — `¥328` rendered directly on the icon, no window needed
- **Aligned multi-line tooltip** — full-width-space alignment keeps value columns crisp (native OS tooltip)
- **Tray hover card** — optional gradient gold card shown on hover (toggle in settings); falls back to the native tooltip
- **Configurable duration format** — `hms` (Xh Xm Xs) / `hm` (Xh Xm) / `h` (decimal hours)
- **Single instance** — launching a second copy focuses the existing window instead
- **Launch on boot** — toggle in settings; backed by the official `tauri-plugin-autostart` plugin (on Windows it writes the current user's Run registry key, no admin rights). The registry is the single source of truth, so disabling it in Task Manager is reflected honestly in the settings page
- **Double-click tray to show** the main/settings window
- **Auto holiday data** — fetches the official China holiday schedule (days off + make-up workdays) from a CDN, caches locally; offline it falls back to a built-in schedule table (2025/2026), and only to Mon–Fri for years not covered yet
- **No runtime dependency** — the release `.exe` is self-contained (WebView2 Runtime is the only system prerequisite, pre-installed on Win10/11)
- **Overtime tracking** — when you lock the screen after the workday, it records your leave time and computes overtime pay + meal allowance; view/edit/delete records per day
- **Activity monitoring** — Raw Input (`WM_INPUT`) captures mouse/keyboard: clicks, scrolls, keystrokes, movement, bucketed per hour with a top-keys leaderboard (no input lag, even with IME)
- **App usage monitoring** — foreground-window hook measures time spent per application (whitelist-based, e.g. WeChat)
- **Media playback monitoring** — audio-session peak polling measures playback time per app
- **Local persistence** — all history (overtime, activity, app/audio usage) is stored in a WAL SQLite database; config & holiday cache remain JSON
- **Storage visibility** — Settings shows total footprint plus a nine-way breakdown (overtime / activity / app usage / media / index & free pages / WAL / icon cache / config & holidays / logs), sorted by size; per-table figures come from SQLite `dbstat`. Retention is configurable (permanent by default) with one-click cleanup, and the WAL is shrunk on first launch each day
- **Week bill** — a new "Bill" tab in the sidebar turns a week's shifts, money and slacking into a page-flippable bill; switch between receipt style and dashboard style in Settings
- **Slacking cost in real time** — the home hero shows how much you have "burned" (¥X · slack rate Y%) right under the ticking earnings, with four tiers of roasting one-liners by slack rate
- **App categories** — app usage is grouped into work / slack / chat / other; click a tag to change it and the whole history is reclassified instantly
- **Cross-midnight overtime** — overnight overtime is attributed to the correct day (locking the screen before 06:00 counts for the previous day with +24h), so nothing is silently lost
- **Weekend & holiday overtime** — driven by official holiday data, with separate start times and pay rates for rest days and statutory holidays
- **History browsing + CSV export** — flip overtime details by month and activity / apps / media by day; export the week bill or overtime details as Excel-friendly CSV (UTF-8 BOM)
- **Storage breakdown & cleanup** — a nine-way storage breakdown in Settings (exact `dbstat` figures), with a retention period and one-click cleanup that shrinks the write-ahead log
- **Remote session detection** — auto overtime recording is skipped while this PC is remotely controlled via RDP / Sunlogin / ToDesk / UU (configurable; manual entry unaffected)

- **Month & year bills** — the Bill tab spans week / month / year; the year view aggregates by month, and payday pushes a notification recapping last month's earnings (can be disabled)
- **Day timeline** — a "Timeline" insight tab: hour-by-hour foreground composition (work/slack/chat/other), top app, input & media activity for any past day; empty hours stay empty — monitor-off never fakes "away"
- **Bill as image** — render the current period's bill into a dark-and-gold report card (income highlights / breakdown rows / daily bars / extremes / quip), copied to clipboard and saved to Downloads
- **Hourly wage mode** — switch pay mode between monthly salary and hourly rate; all downstream money math inherits automatically
- **Focus sessions** — a sixth "Focus" insight tab: segments of continuous work-foreground + keyboard activity (default 25 min, adjustable) counted per day; sleep gaps never counted, accumulates from v1.7.0 on
- **Category suggestions** — one-tap suggested categories for uncategorized apps (offline keyword rules)
- **Monthly bill image** — "save as image" now covers the month span, quip wording follows the period
- **Data insights** — three extra views on the Bill tab: weekday × hour activity heatmap, earnings and slack-rate trends across eight periods, and a period "body bill" of keyboard/mouse wear
- **Guard reminders** — sedentary reminders and off-work reminders via native Windows notifications, plus global hotkeys (Alt+Shift+N toggle window, Alt+Shift+P pause monitoring); configurable thresholds, all off-work aware
- **Manual pause** — pause all monitoring from the tray menu or Alt+Shift+P; money freezes until resumed
- **Six-card settings** — salary & schedule, overtime, guard, data monitoring, appearance, and system & data, each with collapsible detail notes
- **Settings previews and feedback** — six-category navigation, live pay and appearance previews, dependent controls, explicit rate inheritance and retention rules, inline validation, and serialized auto-save.
- **Automatic updates** — checks GitHub Releases every 6 hours (and 30s after launch), one-click update with "skip this version", and a post-update announcement pulled from the changelog; the portable build self-replaces with checksum-verified rollback
- **Backup & restore** — one-click full backup (SQLite snapshot + config + manifest) into a single zip under `Documents\niuma-timer-backup`, list all backups, and one-click restore with integrity validation, automatic safety backup, and auto-restart
- **Build info in startup log** — version, build time, git commit and frontend fingerprint are compiled into the exe and written to `debug.log` / `panic.log` at startup

## How it works

```
hourly rate = monthly salary
            ÷ workdays this month
            ÷ daily working hours

earned today = worked hours × hourly rate
money rate   = hourly rate ÷ 60
```

Lunch break is excluded automatically (morning + afternoon segments configured separately). Earnings cap at the end of the workday; rest days show "day off".

Overtime is detected via the Windows lock-screen event: leaving (locking) the machine after the configured overtime start time records a daily overtime record (`raw_hours` → valid hours floored to 0.5h, fee = valid × rate, plus optional meal allowance).

The overtime day is derived from the **lock timestamp itself**: a leave time before 06:00 counts as past-midnight, so it is attributed to the **previous day** with +24h added to the end time — locking at 01:30 on 9/12 after working through the night is recorded as 7.5h on 9/11, shown as "next day 01:30" in the detail view.

### Backup and restore

Settings → System & Data writes a full backup (database + config + manifest) as a single zip under `Documents\niuma-timer-backup\`. The database snapshot is produced with SQLite's `VACUUM INTO`, which merges the WAL and yields a page-consistent database file — copying the live file would silently drop recent writes. Restore validates the archive (manifest identity, `PRAGMA integrity_check`, all seven business tables), first creates a safety backup of current data, stages `.pending` files, and restarts: the file swap runs after single-instance ownership is acquired and before the database opens, because the app holds one database connection for its whole lifetime and swapping the file under a live connection would mix old and new pages. The previous database is kept as `niuma.db.pre-restore.bak`; a failed swap never blocks startup.

## Requirements

- **Windows 10 / 11**
- **WebView2 Runtime** — usually pre-installed; if missing, download from [Microsoft](https://developer.microsoft.com/microsoft-edge/webview2/)
- Internet access (only for the first holiday-data fetch; offline mode uses local cache)

## Build

### Prerequisites

- [Rust toolchain](https://rustup.rs) (stable, ≥ 1.77)
- The project uses **Tauri v2** (Rust backend + plain HTML/CSS/JS frontend, no npm build step)

### One-click build (recommended)

```bat
build.bat            :: build both debug and release
build.bat debug      :: debug only
build.bat release    :: release only
```

Artifacts are copied to:

- `bin\debug\niuma-timer.exe`
- `bin\release\niuma-timer.exe`

The script auto-detects your `rustc` host triple and locates the output under `target\<triple>\<flavor>\`.

### Manual build

```bash
cd src-tauri
cargo build --release
# Output: src-tauri/target/<triple>/release/niuma-timer.exe
```

### Packaging (NSIS installer / MSI / portable)

Install the Tauri CLI once:

```bash
cargo install tauri-cli --version "^2"
```

Then build everything with one command:

```bat
build.bat package
```

Artifacts land in `bin\package\`:

| Artifact | Notes |
|---|---|
| `*-setup.exe` | NSIS wizard (Chinese UI, per-user install by default, no admin rights) |
| `*.msi` | MSI package for enterprise deployment / silent install |
| `niuma-timer-1.0.0-portable.exe` | Portable exe — no install, no registry writes |

> NSIS is downloaded automatically on first run; MSI additionally needs [WiX 3](https://wixtoolset.org/).

### One-click release (build + package + tag; CI publishes)

```bat
release.bat             use the current version from Cargo.toml
release.bat 1.1.0       also bump the version to 1.1.0 (syncs Cargo.toml + tauri.conf.json)
release.bat 1.1.0 /y    no prompts at all
```

It runs: tests -> release build -> package all three artifacts -> commit and tag -> push.
**Publishing happens in the cloud**: pushing the `vX.Y.Z` tag triggers
`.github/workflows/release.yml`, which rebuilds on GitHub's runners, packages, and creates
the GitHub Release (watch progress on the Actions page).

- Requires `tauri-cli` locally for the pre-flight package; the script offers to install it when missing
- One-time setup: put the updater signing key in repo **Secrets** as `TAURI_SIGNING_PRIVATE_KEY`
  (optionally `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`) — without it the release job fails at the
  updater-manifest check
- Emergency manual publish (CI unavailable) reuses the GitHub token stored by Git Credential
  Manager: `python scripts/publish_release.py --tag vX.Y.Z --version X.Y.Z --package bin/package`
- Bumping the version rewrites `Cargo.toml` and `tauri.conf.json`; if that fails, restore with
  `git checkout --` as printed by the script

### Frontend hot iteration

`dev.bat` serves `frontend/` on the devUrl port (1420) and runs `cargo tauri dev`: the Rust side
compiles once, then editing `index.html` / `styles.css` / `js/*.js` is just a page reload.
Requires Python (static server) and `tauri-cli`.

### Dependency security gate

`cargo deny check advisories` — config in `src-tauri/deny.toml` — runs in CI as the `dep-audit`
job. Unmaintained notices on Linux-only GUI transitive deps are explicitly exempted with a
reason; real CVEs get a dependency bump instead.

### Emergency push (github.com unreachable)

`scripts/push_via_api.py <branch> [--base main]` replays local commits through the GitHub Git Data
API (blob-SHA verified, remote base tree checked) for the times when `github.com` is blocked but
`api.github.com` is not.

### Tests

Run `build.bat test` for the full suite: it runs `cargo test`, `node scripts/run_all.js`
(33 assert scripts) and `python scripts/test_publish_release.py` (the release engine's offline
suite). The frontend assert scripts read the frontend source via `scripts/lib/fe_sources.js`,
which concatenates the 10 blocks under `frontend/js/` in load order before any assertions.
Rust-side source assertions go through `scripts/lib/rs_sources.js`, which mirrors
`src-tauri/src/**/*.rs` recursively (commands live in the `cmds_*.rs` modules since main.rs was
modularized).

Contributor-facing conventions — locking discipline, cross-boundary naming contracts, test rules
and the local environment traps — live in [`docs/CONVENTIONS.md`](./docs/CONVENTIONS.md).


## Usage

1. Run `niuma-timer.exe` — a tray icon appears showing `¥0`.
2. **Right-click the tray → Settings**: enter your monthly salary, morning/afternoon start-end times, and payday. Click **Save**.
3. Click **Refresh workdays** to fetch this year's holidays; the app auto-calculates the actual workday count for the current month (you can also override it manually).
4. The tray icon now refreshes every second with `¥XX`. Hover to see the breakdown; **double-click** the tray icon to open the main window, which shows live wage stats plus overtime / activity / app-usage / media-playback summaries, each with a detail view.

### Config fields

| Field | Description |
|---|---|
| `monthly_salary` | Monthly salary (¥) |
| `salary_mode` | Pay mode: `monthly` (default) or `hourly` |
| `hourly_wage` | Hourly wage (¥/h), effective when `salary_mode` is `hourly` |
| `focus_enabled` | Focus-session tracking switch (default on) |
| `focus_min_minutes` | Focus segment threshold (minutes, 10-120, default 25) |
| `am_start` / `am_end` | Morning work segment (e.g. `09:00` / `12:00`) |
| `pm_start` / `pm_end` | Afternoon work segment (e.g. `13:30` / `18:00`) |
| `payday` | Pay day of each month (1–31) |
| `workdays_override` | Manually override the auto-calculated workday count (optional) |
| `duration_format` | Duration display: `hms` / `hm` / `h` (default `hms`) |
| `tray_hover_card` | Show the colored hover card instead of the native tooltip (default on) |
| `overtime_enabled` | Enable overtime tracking (default off) |
| `overtime_start` | Overtime start time `HH:MM` (empty = `pm_end`) |
| `overtime_rate` | Overtime pay (¥/hour, default 20) |
| `overtime_meal_enabled` | Enable meal allowance (default on) |
| `overtime_meal` | Meal allowance amount (¥, default 20) |
| `weekend_overtime` | Count rest-day / public-holiday overtime (default off) |
| `weekend_ot_start` | Rest-day overtime start `HH:MM` (default `09:00`; a rest day has no clock-off time) |
| `overtime_rate_weekend` | Rest-day rate (¥/hour; falls back to `overtime_rate`) |
| `overtime_rate_holiday` | Public-holiday rate (¥/hour; falls back to the weekend rate) |
| `monitor_activity` | Mouse/keyboard activity monitoring (default on) |
| `monitor_app_usage` | App usage monitoring (default on) |
| `monitor_audio` | Media playback monitoring (default on) |

The table lists the knobs people actually change; the full set (reminders, appearance,
updates, retention) is whatever `config.json` contains.

## Data storage

- Config: `%APPDATA%\niuma-timer\config.json`
- Holiday cache: `%APPDATA%\niuma-timer\holiday_{year}.json`
- History DB: `%APPDATA%\niuma-timer\niuma.db` (SQLite, WAL)
- Holiday data source: [`NateScarlet/holiday-cn`](https://github.com/NateScarlet/holiday-cn) via jsDelivr CDN (official State Council schedule, including make-up workdays)

## Tech stack

| Layer | Technology |
|---|---|
| Backend | Rust + Tauri v2 |
| Frontend | Vanilla HTML / CSS / JS (no bundler) |
| Single instance | `tauri-plugin-single-instance` |
| Tray icon | Runtime pixel rendering with a built-in 5×7 dot-matrix font |
| Persistence | SQLite (`rusqlite`, WAL mode) |
| Windows APIs | `windows` crate 0.61 — WTS lock-screen events, audio-session polling, DPI-aware window icon, Raw Input (WM_INPUT) for mouse/keyboard |
| Holiday data | GitHub-hosted JSON via jsDelivr CDN, with local cache + built-in schedule fallback |

## Known limitations

- **Rest-day / public-holiday overtime** is off by default; enable it in settings to have those days counted. Rest days start counting from 09:00 by default (a rest day has no "clock-off time"), and the rate can be configured separately, falling back by level when unset (public holiday -> rest day -> workday). Recognising public holidays and makeup workdays depends on holiday data; without data for that year it falls back to the day of week.
- Monitor threads consume a small amount of CPU while running; disable unused ones in settings (already-counted data is kept).
- Tooltip (native mode) is plain text rendered by the OS (no colors/icons); alignment relies on full-width-space (U+3000) column math.
- Cross-platform (macOS/Linux) is not supported.

## Project structure

```
niuma-timer/
├── src-tauri/
│   ├── src/
│   │   ├── main.rs         # Entry & wiring: plugin setup, boot sequence, scheduler/tray hooks, command registration
│   │   ├── state.rs        # Shared AppState + today's status snapshot
│   │   ├── diag.rs         # Startup diagnostics: panic.log / boot traces / build info / fatal-error dialog
│   │   ├── cmds_core.rs    # Commands: config / holidays / window / autostart / CSV + monitor switches & shortcuts
│   │   ├── cmds_bill.rs    # Commands: overtime records + week/month/year billing + insights
│   │   ├── cmds_monitor.rs # Commands: mouse-keyboard / app / media summary queries
│   │   ├── cmds_storage.rs # Commands: storage usage / manual maintenance / backup & restore
│   │   ├── cmds_update.rs  # Commands: check update / one-click update (installed & portable) / skip / announcement
│   │   ├── cmds_debug.rs   # Commands: debug card (notify test / instant tick / simulated sedentary)
│   │   ├── config.rs       # Config struct + load/save
│   │   ├── calc.rs         # Earnings calc + duration formatting + tooltip
│   │   ├── holiday.rs      # Holiday data fetch + parse + cache + built-in fallback table
│   │   ├── icon_render.rs  # Tray icon pixel rendering
│   │   ├── tray.rs         # Tray icon, menu, hover card (debounce/watchdog/click-cooldown)
│   │   ├── tray/hover_state.rs  # Hover-card interaction state machine (pure)
│   │   ├── db.rs           # SQLite layer (WAL), schema init
│   │   ├── overtime.rs     # Overtime records: calc + persistence
│   │   ├── weekbill.rs     # Week/month/year bill aggregation
│   │   ├── insights.rs     # Insights aggregation (heatmap / trend / body bill)
│   │   ├── focus.rs        # Focus-session state machine (v1.7.0)
│   │   ├── lock_monitor.rs # Windows lock-screen listener (WTS session change)
│   │   ├── activity.rs     # Raw Input mouse/keyboard capture + hourly buckets + top keys
│   │   ├── app_usage.rs    # Foreground-window app usage tracking (whitelist)
│   │   ├── audio_usage.rs  # Audio-session media playback tracking
│   │   ├── remind.rs       # Sedentary / end-of-work reminders via system notifications
│   │   ├── pause.rs        # Manual pause state machine
│   │   ├── remote.rs       # Remote-session detection (RDP / Sunlogin / ToDesk / UU)
│   │   ├── backup.rs       # Backup & restore (VACUUM INTO snapshot + startup swap)
│   │   ├── update.rs       # Auto-update (check / download / verify / built-in helper swap)
│   │   ├── win.rs          # Win32 platform layer (RAII guards: window/hooks/COM/GDI, process & icon, raw-input parsing)
│   │   ├── scheduler.rs    # Unified periodic scheduler (1s beat -> 1s/5s/10s tasks)
│   │   ├── sync.rs         # Lock access gateway (poison recovery; no bare .lock().unwrap())
│   │   └── maintain.rs     # Data lifecycle (usage stats / WAL shrink / expired data & icon cleanup)
│   ├── Cargo.toml
│   ├── build.rs           # Tauri build (registers commands)
│   ├── tauri.conf.json
│   ├── capabilities/default.json
│   ├── deny.toml          # cargo-deny config (CI dep-audit gate)
│   └── permissions/autogenerated/  # command ACL manifests
├── frontend/
│   ├── index.html         # 8 views: home / bill / settings / update + 4 detail pages
│   ├── js/                # 10 classic scripts, loaded by index.html in this order
│   │   ├── core.js        # shared helpers, state & invoke shim (loads first)
│   │   ├── settings.js    # settings page
│   │   ├── hero.js        # home hero: live wage display
│   │   ├── overtime.js    # overtime records view
│   │   ├── monitor.js     # activity / app-usage / media views
│   │   ├── bill.js        # week / month / year bill tab
│   │   ├── insights.js    # data insights views
│   │   ├── storage.js     # system & data card: storage usage, cleanup, backups
│   │   ├── update.js      # auto update
│   │   └── boot.js        # view switching + startup sequence (loads last)
│   ├── styles.css
│   └── hover_card.html    # tray hover card (independent document with its own state)
├── scripts/               # test suites + release tooling
│   ├── test_*.js          # 33 source-contract scripts, collected by scripts/run_all.js
│   ├── test_publish_release.py  # release-engine offline suite (also runs in CI)
│   ├── lib/               # fe_sources.js / rs_sources.js source aggregators
│   ├── publish_release.py # GitHub Release engine (CI uses this too)
│   └── push_via_api.py    # emergency push when github.com is unreachable
├── common.bat             # shared setup: build-owned TMP, vcvars/RC seeding, cargo fallback
├── build.bat              # one-click build / test (debug + release → bin/)
├── dev.bat                # frontend hot iteration
├── release.bat            # version bump / test / package / tag / push (CI publishes)
├── CHANGELOG.md
├── docs/CONVENTIONS.md    # contributor-facing conventions
└── bin/                   # Build artifacts (gitignored)
```

---

Made with 🦀 by a fellow wage worker.
