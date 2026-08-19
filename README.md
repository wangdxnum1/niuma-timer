# NiuMa Timer

> Track how much you've earned today, down to the second.
> [中文文档](./README.zh-CN.md)

A lightweight Windows system-tray tool for wage workers. It sits in your tray and shows, in real time:

- How much you've earned **today** (¥)
- How long you've been **working** (hours/minutes/seconds)
- How long **until you're off work**
- Your **money rate** (¥/min) and **days until payday**

The tray icon itself renders the earned amount — just glance at the taskbar to feel the ¥/sec.

## Features

- **Today-only accounting** — resets at midnight, no carry-over
- **Tray icon draws the amount** — `¥328` rendered directly on the icon, no window needed
- **Aligned multi-line tooltip** — full-width-space alignment keeps value columns crisp:

  ```
  今日已赚　¥328.50
  已工作　　5小时10分30秒
  距下班　　1小时50分30秒
  ────────
  赚钱速率　¥5.48/分
  距发薪　　23天
  ```

- **Configurable duration format** — `hms` (Xh Xm Xs) / `hm` (Xh Xm) / `h` (decimal hours)
- **Single instance** — launching a second copy focuses the existing window instead
- **Double-click tray to show** the main/settings window
- **Auto holiday data** — fetches the official China holiday schedule (days off + make-up workdays) from a CDN, caches locally; falls back to Mon–Fri when offline
- **No runtime dependency** — the release `.exe` is self-contained (WebView2 Runtime is the only system prerequisite, pre-installed on Win10/11)

## How it works

```
时薪 (hourly rate) = 月薪 (monthly salary)
                   ÷ 当月实际工作日 (workdays this month)
                   ÷ 每日工时 (daily working hours)

今日已赚 (earned today) = 已工作小时 × 时薪
赚钱速率 (¥/min)       = 时薪 ÷ 60
```

Lunch break is excluded automatically (morning + afternoon segments configured separately). Earnings cap at the end of the workday; rest days show "今天休息".

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
4. The tray icon now refreshes every second with `¥XX`. Hover to see the full aligned breakdown; **double-click** the tray icon to open the main window.

### Config fields

| Field | Description |
|---|---|
| `monthly_salary` | Monthly salary (¥) |
| `am_start` / `am_end` | Morning work segment (e.g. `09:00` / `12:00`) |
| `pm_start` / `pm_end` | Afternoon work segment (e.g. `13:30` / `18:00`) |
| `payday` | Pay day of each month (1–31) |
| `workdays_override` | Manually override the auto-calculated workday count (optional) |
| `duration_format` | Duration display: `hms` / `hm` / `h` (default `hms`) |

## Data storage

- Config: `%APPDATA%\niuma-timer\config.json`
- Holiday cache: `%APPDATA%\niuma-timer\holiday_{year}.json`
- Holiday data source: [`NateScarlet/holiday-cn`](https://github.com/NateScarlet/holiday-cn) via jsDelivr CDN (official State Council schedule, including make-up workdays)

## Tech stack

| Layer | Technology |
|---|---|
| Backend | Rust + Tauri v2 |
| Frontend | Vanilla HTML / CSS / JS (no bundler) |
| Single instance | `tauri-plugin-single-instance` |
| Tray icon | Runtime pixel rendering with a built-in 5×7 dot-matrix font |
| Holiday data | GitHub-hosted JSON via jsDelivr CDN, with local cache fallback |

## Known limitations

- No overtime tracking — earnings cap at end of workday.
- Rest days show "今天休息" and accumulate nothing.
- Tooltip is plain text rendered by the OS (no colors/icons); alignment relies on full-width-space (U+3000) column math.

## Project structure

```
niuma-timer/
├── src-tauri/
│   ├── src/
│   │   ├── main.rs        # App bootstrap, invoke shim, timer loop
│   │   ├── config.rs      # Config struct + load/save
│   │   ├── calc.rs        # Earnings calc + duration formatting + tooltip
│   │   ├── holiday.rs     # Holiday data fetch + parse + cache
│   │   ├── icon_render.rs # Tray icon pixel rendering
│   │   └── tray.rs        # Tray icon, menu, double-click handler
│   ├── Cargo.toml
│   ├── build.rs           # Tauri build (registers commands)
│   ├── tauri.conf.json
│   └── capabilities/default.json
├── frontend/
│   ├── index.html
│   ├── app.js
│   └── styles.css
├── build.bat              # One-click build (debug + release → bin/)
└── bin/                   # Build artifacts (gitignored)
```

---

Made with 🦀 by a fellow 牛马.
