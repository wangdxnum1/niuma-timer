# NiuMa Timer

> Track how much you've earned today, down to the second — plus what you actually did all day.
> [中文文档](./README.zh-CN.md)

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
- **Double-click tray to show** the main/settings window
- **Auto holiday data** — fetches the official China holiday schedule (days off + make-up workdays) from a CDN, caches locally; falls back to Mon–Fri when offline
- **No runtime dependency** — the release `.exe` is self-contained (WebView2 Runtime is the only system prerequisite, pre-installed on Win10/11)
- **Overtime tracking** — when you lock the screen after the workday, it records your leave time and computes overtime pay + meal allowance; view/edit/delete records per day
- **Activity monitoring** — Raw Input (`WM_INPUT`) captures mouse/keyboard: clicks, scrolls, keystrokes, movement, bucketed per hour with a top-keys leaderboard (no input lag, even with IME)
- **App usage monitoring** — foreground-window hook measures time spent per application (whitelist-based, e.g. WeChat)
- **Media playback monitoring** — audio-session peak polling measures playback time per app
- **Local persistence** — all history (overtime, activity, app/audio usage) is stored in a WAL SQLite database; config & holiday cache remain JSON

## How it works

```
hourly rate = monthly salary
            ÷ workdays this month
            ÷ daily working hours

earned today = worked hours × hourly rate
money rate   = hourly rate ÷ 60
```

Lunch break is excluded automatically (morning + afternoon segments configured separately). Earnings cap at the end of the workday; rest days show "今天休息" (day off).

Overtime is detected via the Windows lock-screen event: leaving (locking) the machine after the configured overtime start time records a daily overtime record (`raw_hours` → valid hours floored to 0.5h, fee = valid × rate, plus optional meal allowance).

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

### Build an installer (.msi)

```bash
cargo install tauri-cli --version "^2"
cargo tauri build
```

## Usage

1. Run `niuma-timer.exe` — a tray icon appears showing `¥0`.
2. **Right-click the tray → Settings** (设置): enter your monthly salary, morning/afternoon start-end times, and payday. Click **Save** (保存).
3. Click **Refresh workdays** (刷新工作日数据) to fetch this year's holidays; the app auto-calculates the actual workday count for the current month (you can also override it manually).
4. The tray icon now refreshes every second with `¥XX`. Hover to see the breakdown; **double-click** the tray icon to open the main window, which shows live wage stats plus overtime / activity / app-usage / media-playback summaries, each with a detail view.

### Config fields

| Field | Description |
|---|---|
| `monthly_salary` | Monthly salary (¥) |
| `am_start` / `am_end` | Morning work segment (e.g. `09:00` / `12:00`) |
| `pm_start` / `pm_end` | Afternoon work segment (e.g. `13:30` / `18:00`) |
| `payday` | Pay day of each month (1–31) |
| `workdays_override` | Manually override the auto-calculated workday count (optional) |
| `duration_format` | Duration display: `hms` / `hm` / `h` (default `hms`) |
| `tray_hover_card` | Show the colored hover card instead of the native tooltip (default off) |
| `overtime_enabled` | Enable overtime tracking (default off) |
| `overtime_start` | Overtime start time `HH:MM` (empty = `pm_end`) |
| `overtime_rate` | Overtime pay (¥/hour, default 20) |
| `overtime_meal_enabled` | Enable meal allowance (default on) |
| `overtime_meal` | Meal allowance amount (¥, default 20) |
| `monitor_activity` | Mouse/keyboard activity monitoring (default on) |
| `monitor_app_usage` | App usage monitoring (default on) |
| `monitor_audio` | Media playback monitoring (default on) |

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
| Holiday data | GitHub-hosted JSON via jsDelivr CDN, with local cache fallback |

## Known limitations

- **Weekend overtime** is reserved in config (`weekend_overtime`) but not yet implemented.
- Monitor threads consume a small amount of CPU while running; disable unused ones in settings (already-counted data is kept).
- Tooltip (native mode) is plain text rendered by the OS (no colors/icons); alignment relies on full-width-space (U+3000) column math.
- Cross-platform (macOS/Linux) is not supported.

## Project structure

```
niuma-timer/
├── src-tauri/
│   ├── src/
│   │   ├── main.rs         # App bootstrap, invoke shim, 1s timer loop, overtime lock detection
│   │   ├── config.rs       # Config struct + load/save
│   │   ├── calc.rs         # Earnings calc + duration formatting + tooltip
│   │   ├── holiday.rs      # Holiday data fetch + parse + cache
│   │   ├── icon_render.rs  # Tray icon pixel rendering
│   │   ├── tray.rs         # Tray icon, menu, hover card (debounce/watchdog/click-cooldown)
│   │   ├── db.rs           # SQLite layer (WAL), schema init
│   │   ├── overtime.rs     # Overtime records: calc + persistence
│   │   ├── lock_monitor.rs # Windows lock-screen listener (WTS session change)
│   │   ├── activity.rs     # Raw Input mouse/keyboard capture + hourly buckets + top keys
│   │   ├── app_usage.rs    # Foreground-window app usage tracking (whitelist)
│   │   └── audio_usage.rs  # Audio-session media playback tracking
│   ├── Cargo.toml
│   ├── build.rs           # Tauri build (registers commands)
│   ├── tauri.conf.json
│   ├── capabilities/default.json
│   └── permissions/autogenerated/  # command ACL manifests
├── frontend/
│   ├── index.html         # 6 views: main / settings / overtime / activity / app-usage / media
│   ├── app.js
│   └── styles.css
├── build.bat             # One-click build (debug + release → bin/)
└── bin/                  # Build artifacts (gitignored)
```

---

Made with 🦀 by a fellow 牛马.
