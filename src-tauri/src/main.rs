#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod activity;
mod app_usage;
mod audio_usage;
mod calc;
mod config;
mod db;
mod holiday;
mod icon_render;
mod lock_monitor;
mod overtime;
mod tray;
mod win;

use std::io::Write;
use std::panic;
use std::sync::Mutex;

use chrono::{Datelike, Local, NaiveDate, TimeZone};
use serde_json::{from_value, to_value, Value};
use tauri::{Manager, State};

struct AppState {
    config: Mutex<config::Config>,
    holiday: Mutex<holiday::HolidayCache>,
    last_date: Mutex<NaiveDate>,
    /// 最近一次已处理的锁屏时间戳，用于检测新锁屏事件
    last_lock_seen: Mutex<Option<i64>>,
}

impl Default for AppState {
    fn default() -> Self {
        AppState {
            config: Mutex::new(config::load()),
            holiday: Mutex::new(holiday::HolidayCache::default()),
            last_date: Mutex::new(Local::now().date_naive()),
            last_lock_seen: Mutex::new(None),
        }
    }
}

/// 启动崩溃诊断：进程早期（webview 未起）崩溃时前端 debug.log 无效，
/// 故把 panic 与启动阶段痕迹单独写到 %APPDATA%/niuma-timer/panic.log。
/// release 无控制台窗口，panic 会静默退出，此文件是排查「双击无反应」的唯一线索。
fn panic_log_path() -> std::path::PathBuf {
    config::config_dir().join("panic.log")
}

/// 追加一行启动阶段痕迹到 panic.log（每次启动先由 install_crash_log 清空重写）。
fn trace_startup(stage: &str) {
    let _ = std::fs::create_dir_all(config::config_dir());
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(panic_log_path())
    {
        let line = format!("[{}] {stage}\n", Local::now().format("%Y-%m-%d %H:%M:%S%.3f"));
        let _ = f.write_all(line.as_bytes());
    }
}

/// 安装 panic hook：捕获 main() 启动期任何 panic，写 message + backtrace 到 panic.log，
/// 让「进程起来又退出」的场景可定位。必须在 main() 最开头调用。
fn install_crash_log() {
    let _ = std::fs::create_dir_all(config::config_dir());
    let _ = std::fs::write(
        panic_log_path(),
        format!("[start] {}\n", Local::now().format("%Y-%m-%d %H:%M:%S%.3f")),
    );
    panic::set_hook(Box::new(|info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let line = format!("[panic] {info}\n{bt:?}\n");
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(panic_log_path())
            .and_then(|mut f| f.write_all(line.as_bytes()));
    }));
}

/// 计算当月实际上班天数（手动覆盖 > 缓存 > 兜底周末数）
fn current_monthly_workdays(cfg: &config::Config, hol: &holiday::HolidayCache) -> u32 {
    if let Some(n) = cfg.workdays_override {
        return n;
    }
    let now = Local::now();
    if let Some(n) = hol.month_workdays(now.year(), now.month()) {
        return n;
    }
    holiday::weekday_count(now.year(), now.month())
}

/// 计算当天状态快照
pub(crate) fn get_status(state: &AppState) -> calc::DayStatus {
    let cfg = state.config.lock().unwrap().clone();
    let hol = state.holiday.lock().unwrap().clone();
    let now = Local::now();
    let is_workday = hol.is_workday(now.date_naive()).unwrap_or_else(|| {
        let wd = now.weekday().num_days_from_monday();
        wd < 5
    });
    let mw = current_monthly_workdays(&cfg, &hol);
    calc::compute(&cfg, is_workday, mw, now)
}

/// 后台拉取并刷新节假日缓存
pub fn spawn_holiday_refresh(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let year = Local::now().year();
        match holiday::fetch_year(year).await {
            Ok(days) => {
                let state = app.state::<AppState>();
                let mut hol = state.holiday.lock().unwrap();
                hol.year = year;
                hol.days = days;
                hol.fetched_at = Local::now().timestamp();
                let hol_clone = hol.clone();
                drop(hol);
                holiday::save_cache(&hol_clone);
                let st = get_status(state.inner());
                tray::update_tray(&app, &st);
            }
            Err(e) => eprintln!("节假日刷新失败: {}", e),
        }
    });
}

#[tauri::command]
fn load_config(state: State<AppState>) -> config::Config {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn save_config(state: State<AppState>, app: tauri::AppHandle, cfg: Value) {
    // 合并保存：以现有配置为基底，仅用前端传来的字段覆盖，保留前端未管理的字段
    // （如 last_holiday_year 等保留字段，以及未来新增字段），避免整份替换把未传字段重置成默认值。
    let existing = state.config.lock().unwrap().clone();
    let mut base = to_value(&existing).unwrap_or(Value::Null);
    if let Some(obj) = base.as_object_mut() {
        if let Some(incoming) = cfg.as_object() {
            for (k, v) in incoming {
                obj.insert(k.clone(), v.clone());
            }
        }
    }
    let merged: config::Config = match from_value(base) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[config] save_config 合并失败，保留原配置: {e}");
            return;
        }
    };
    *state.config.lock().unwrap() = merged.clone();
    config::save(&merged);
    // 监控开关即时生效（关闭前先结算已累计的应用使用时长）
    apply_monitor_switches(&merged);
    let st = get_status(state.inner());
    tray::update_tray(&app, &st);
}

/// 把配置里的三个监控开关同步到各监控模块（启动时与保存配置后调用）。
/// app_usage 在关闭前先 tick() 结算一次，避免丢掉最后一段已使用时长。
fn apply_monitor_switches(cfg: &config::Config) {
    if !cfg.monitor_app_usage {
        app_usage::shutdown();
    }
    activity::set_enabled(cfg.monitor_activity);
    app_usage::set_enabled(cfg.monitor_app_usage);
    audio_usage::set_enabled(cfg.monitor_audio);
    // 应用使用白名单（开启后只统计名单内应用）
    app_usage::set_whitelist(cfg.app_whitelist_enabled, cfg.app_whitelist.clone());
    // 重开时把当前前台窗口立即纳入统计
    if cfg.monitor_app_usage {
        app_usage::refresh_foreground();
    }
}

/// 每秒刷新托盘实时状态（已赚¥ / 距下班 / 距发薪日）。仅 UI 刷新，无副作用，
/// 从主循环抽离以便独立调频率或单元测试。
fn refresh_tray(apph: &tauri::AppHandle, state: &AppState) {
    let st = get_status(state);
    tray::update_tray(apph, &st);
}

/// 跨天检测：日期变化（last_date 落后）时重拉当年节假日缓存。
/// 低频调用即可，无需秒级——托盘 UI 每秒仍按实时日期正常显示。
fn maybe_rollover_day(state: &AppState, apph: &tauri::AppHandle) {
    let today = Local::now().date_naive();
    let need_refresh = {
        let mut last = state.last_date.lock().unwrap();
        if *last != today {
            *last = today;
            true
        } else {
            false
        }
    };
    if need_refresh {
        spawn_holiday_refresh(apph.clone());
    }
}

/// 加班锁屏检测：last_lock_timestamp 变化且开启加班、且为工作日时，计算并落盘加班记录。
/// 原耦合在 1s 主循环里每秒轮询，现抽到低频业务线程，减少无谓的每秒锁竞争与计算。
fn maybe_record_overtime_lock(state: &AppState) {
    let cfg = state.config.lock().unwrap().clone();
    if !cfg.overtime_enabled {
        return;
    }
    let Some(lock_ts) = lock_monitor::last_lock_timestamp() else {
        return;
    };
    let mut seen = state.last_lock_seen.lock().unwrap();
    if *seen == Some(lock_ts) {
        return;
    }
    *seen = Some(lock_ts);
    drop(seen);

    let now = Local::now();
    let is_workday = state
        .holiday
        .lock()
        .unwrap()
        .is_workday(now.date_naive())
        .unwrap_or_else(|| now.weekday().num_days_from_monday() < 5);
    if !is_workday {
        return;
    }
    if let Some(lt) = Local.timestamp_opt(lock_ts, 0).single() {
        if let Some(record) = overtime::calc_record(now.date_naive(), lt, &cfg) {
            // 自动路径：只覆盖同为自动来源的记录，用户手改过的那天不会被顶掉
            overtime::upsert_auto(record);
        }
    }
}

#[tauri::command]
async fn refresh_holidays(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<u32, String> {
    let year = Local::now().year();
    let days = holiday::fetch_year(year).await?;
    let mut hol = state.holiday.lock().unwrap();
    hol.year = year;
    hol.days = days;
    hol.fetched_at = Local::now().timestamp();
    let hol_clone = hol.clone();
    drop(hol);
    holiday::save_cache(&hol_clone);
    let mw = current_monthly_workdays(&*state.config.lock().unwrap(), &hol_clone);
    let st = get_status(state.inner());
    tray::update_tray(&app, &st);
    Ok(mw)
}

#[tauri::command]
fn get_status_cmd(state: State<AppState>) -> calc::DayStatus {
    get_status(state.inner())
}

/// 隐藏主窗口（点关闭按钮时调用）
#[tauri::command]
fn hide_window(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

/// 显示主窗口：还原最小化 + 显示 + 抢焦点
/// （最小化状态下 show() 是无效操作，必须先 unminimize）
#[tauri::command]
fn show_window(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// 仅把主窗口提到前台
#[tauri::command]
fn focus_window(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// 获取当月加班记录（含预计算汇总字段）
#[tauri::command]
fn get_overtime_records() -> overtime::MonthlyOvertimeView {
    let now = Local::now();
    overtime::get_month(now.year(), now.month()).to_view()
}

/// 手动添加/修改某天加班记录（仅当月），返回刷新后的当月视图
#[tauri::command]
fn save_overtime_record(
    state: State<AppState>,
    input: overtime::ManualOvertimeInput,
) -> Result<overtime::MonthlyOvertimeView, String> {
    let cfg = state.config.lock().unwrap().clone();
    overtime::save_manual(input, &cfg)?;
    let now = Local::now();
    Ok(overtime::get_month(now.year(), now.month()).to_view())
}

/// 手动删除某天加班记录（仅当月），返回刷新后的当月视图
#[tauri::command]
fn delete_overtime_record(
    date: String,
) -> Result<overtime::MonthlyOvertimeView, String> {
    overtime::delete_manual(&date)?;
    let now = Local::now();
    Ok(overtime::get_month(now.year(), now.month()).to_view())
}

/// 获取今日鼠标/键盘活动统计（逐小时 + 汇总 + 高频按键）
#[tauri::command]
fn get_activity_summary() -> activity::ActivitySummary {
    activity::summary()
}

/// 获取今日应用使用时长统计（各应用累计 + 24 小时分布）
#[tauri::command]
fn get_app_usage_summary() -> app_usage::AppUsageSummary {
    app_usage::summary()
}

/// 获取今日媒体播放时长统计（各应用累计 + 24 小时分布）
#[tauri::command]
fn get_audio_usage_summary() -> audio_usage::AudioUsageSummary {
    audio_usage::summary()
}

/// 前端调试日志落盘（写入 %APPDATA%/niuma-timer/debug.log，排查用户桌面环境用）
#[tauri::command]
fn write_debug_log(msg: String) {
    crate::db::debug_log(&msg);
}

/// 注入到 webview 的轻量 Tauri API 垫片。
/// 本版本未启用全局 window.__TAURI__，这里基于始终存在的
/// window.__TAURI_INTERNALS__.invoke 自行暴露 core.invoke 与 window 控制，
/// 省去前端打包 @tauri-apps/api 的步骤。
const INVOKE_SHIM: &str = r#"
if (!window.__TAURI__) {
  window.__TAURI__ = {
    core: {
      invoke: function (cmd, args) {
        return window.__TAURI_INTERNALS__.invoke(cmd, args || {});
      }
    },
    window: {
      getCurrentWindow: function () {
        return {
          hide: function () { return window.__TAURI_INTERNALS__.invoke('hide_window'); },
          show: function () { return window.__TAURI_INTERNALS__.invoke('show_window'); },
          setFocus: function () { return window.__TAURI_INTERNALS__.invoke('focus_window'); }
        };
      }
    }
  };
}
"#;

/// 修复任务栏图标模糊：
/// tauri-codegen 生成默认窗口图标时只解码 icon.ico 的**第一个图层**（本项目为 16×16），
/// 底层 tao 又把这同一张小图设为 ICON_BIG——任务栏在高 DPI 下放大 16px 位图必然发糊。
/// （托盘图标是运行时 SDF 动态绘制、资源管理器读的是完整多尺寸 ico，所以那两处清晰。）
/// 此处改为从 exe 内嵌的多尺寸 ico 资源（tauri-build 固定 ID 32512）按窗口实际 DPI
/// 分别加载 ICON_BIG / ICON_SMALL 并 WM_SETICON 覆盖，Windows 自动挑选最贴合的图层。
#[cfg(windows)]
unsafe fn set_window_icons_from_resource(hwnd: windows::Win32::Foundation::HWND) {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HINSTANCE, LPARAM, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
    use windows::Win32::UI::WindowsAndMessaging::{
        LoadImageW, SendMessageW, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTSIZE, SM_CXICON,
        SM_CXSMICON, SM_CYICON, SM_CYSMICON, WM_SETICON,
    };

    let Ok(hmod) = GetModuleHandleW(None) else {
        return;
    };
    let hinst = HINSTANCE(hmod.0);
    // MAKEINTRESOURCEW(32512)
    let name = PCWSTR(32512usize as *const u16);
    let mut dpi = GetDpiForWindow(hwnd);
    if dpi == 0 {
        dpi = 96;
    }

    let set_icon = |wparam: u32, cx: _, cy: _| {
        let (cx, cy) = (
            GetSystemMetricsForDpi(cx, dpi).max(1),
            GetSystemMetricsForDpi(cy, dpi).max(1),
        );
        if let Ok(h) = LoadImageW(Some(hinst), name, IMAGE_ICON, cx, cy, LR_DEFAULTSIZE) {
            SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(wparam as usize)),
                Some(LPARAM(h.0 as isize)),
            );
        }
    };
    set_icon(ICON_BIG, SM_CXICON, SM_CYICON);
    set_icon(ICON_SMALL, SM_CXSMICON, SM_CYSMICON);
}

fn main() {
    install_crash_log();
    let app = tauri::Builder::default()
        // 单例模式：若已有实例在运行，第二个实例启动时被拦截，
        // 并在回调里把已存在的主窗口显示并置前，自己退出。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .append_invoke_initialization_script(INVOKE_SHIM)
        .manage(AppState::default())
        .setup(|app| {
            trace_startup("setup: enter");
            // 初始化 SQLite（WAL + 建表）并一次性迁移旧 JSON 数据。
            // 必须在 activity::start() 之前：start 内部 load_today 要从 SQLite 恢复当天统计。
            db::migrate_legacy();
            trace_startup("setup: db migrated");

            // 历史应用名归一化（英文 FileDescription → 中文常用名），
            // 必须在 app_usage::start() / audio_usage::start() 写入新数据之前。
            db::normalize_app_names();
            trace_startup("setup: app names normalized");

            // 载入本地节假日缓存
            {
                let year = Local::now().year();
                if let Some(c) = holiday::load_cache(year) {
                    *app.state::<AppState>().holiday.lock().unwrap() = c;
                }
            }
            trace_startup("setup: holiday cache loaded");
            // 修复任务栏图标模糊：用 exe 内嵌多尺寸 ico 按 DPI 重新设置窗口图标
            #[cfg(windows)]
            if let Some(w) = app.get_webview_window("main") {
                if let Ok(hwnd) = w.hwnd() {
                    unsafe { set_window_icons_from_resource(hwnd) };
                }
            }
            trace_startup("setup: window icons set");

            // 创建托盘
            let _tray = tray::create_tray(app)?;
            trace_startup("setup: tray created");

            // 启动页防白闪：窗口初始 visible:false，由前端 splash 渲染完成后 show；
            // 此处兜底——1 秒后无论如何 show，避免前端 JS 异常导致窗口永久不可见
            {
                let app2 = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(1000));
                    if let Some(w) = app2.get_webview_window("main") {
                        let _ = w.show();
                    }
                });
            }

            // 启动锁屏监听线程（Windows session notification）
            lock_monitor::start();
            trace_startup("setup: lock monitor started");

            // 按配置初始化三个监控开关。活动监控关闭时会真正注销 Raw Input 设备
            // （而非回调里空转），故必须在 activity::start() 之前调用。
            apply_monitor_switches(&app.state::<AppState>().config.lock().unwrap().clone());
            trace_startup("setup: monitor switches applied");

            // 启动鼠标/键盘活动统计（Raw Input 旁路采集，常驻托盘即持续统计，输入法零延迟）
            activity::start();
            trace_startup("setup: activity started");

            // 启动应用使用时长监控（前台窗口事件钩子，统计微信等白名单应用）
            app_usage::start();
            trace_startup("setup: app_usage started");

            // 启动媒体播放时长监控（音频会话峰值轮询，统计正在发声的软件）
            audio_usage::start();
            trace_startup("setup: audio_usage started");

            // 窗口关闭仅隐藏，不退出程序
            if let Some(w) = app.get_webview_window("main") {
                w.clone().on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = w.hide();
                    }
                });
            }

            let apph = app.handle().clone();
            // 启动即拉一次节假日
            spawn_holiday_refresh(apph.clone());

            // 托盘 UI 刷新：独立 1s 循环，仅做实时状态显示（已赚¥/距下班/距发薪），
            // 与下方业务轮询解耦，方便单独调频率或替换实现。
            {
                let apph = apph.clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    let state = apph.state::<AppState>();
                    refresh_tray(&apph, state.inner());
                });
            }

            // 业务轮询：跨天检测 + 加班锁屏检测，低频（无需秒级）。
            // 与 UI 刷新分离后，锁屏→记加班的延迟从 ≤1s 放宽到 ≤5s，对加班统计无影响。
            {
                let apph = apph.clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    let state = apph.state::<AppState>();
                    maybe_rollover_day(state.inner(), &apph);
                    maybe_record_overtime_lock(state.inner());
                });
            }
            trace_startup("setup: done");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_config,
            save_config,
            refresh_holidays,
            get_status_cmd,
            hide_window,
            show_window,
            focus_window,
            get_overtime_records,
            save_overtime_record,
            delete_overtime_record,
            get_activity_summary,
            get_app_usage_summary,
            get_audio_usage_summary,
            write_debug_log
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
    ;
    trace_startup("app: built, entering run loop");
    app.run(|_app_handle, event| {
            // 程序退出前：把鼠标/键盘统计与应用使用时长的最后增量落盘，重启后不丢数据
            if let tauri::RunEvent::Exit = event {
                activity::shutdown();
                app_usage::shutdown();
                audio_usage::shutdown();
            }
        });
    // 前端资源自动重编：build.rs 用 frontend 目录内容指纹注入
    // `cargo:rustc-env=TAURI_FRONTEND_FP`，内容一变即触发本 crate 重编、
    // generate_context! 重跑并重新嵌入最新 HTML/CSS/JS，无需手动改动任何标记。
}
