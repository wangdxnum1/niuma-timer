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
mod maintain;
mod overtime;
mod scheduler;
mod sync;
mod tray;
mod win;

use std::io::Write;
use std::panic;
use std::sync::Mutex;

use chrono::{Datelike, Local, NaiveDate, TimeZone};
use serde_json::{from_value, to_value, Value};
use tauri::{Manager, State};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

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

/// 启动致命错误：弹系统消息框（release 无控制台，必须给可见反馈），同时写 panic.log。
/// 把「双击无反应 / 静默退出」转成可操作的错误提示（尤其是缺 WebView2 的场景）。
#[cfg(windows)]
fn show_fatal(msg: &str) {
    let _ = std::fs::create_dir_all(config::config_dir());
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(panic_log_path())
    {
        let _ = f.write_all(format!("[fatal] {msg}\n").as_bytes());
    }
    crate::win::message_box("牛马计时器", msg);
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
    let cfg = sync::lock(&state.config, "state.config").clone();
    let hol = sync::lock(&state.holiday, "state.holiday").clone();
    let now = Local::now();
    let is_workday = hol.is_workday(now.date_naive()).unwrap_or_else(|| {
        let wd = now.weekday().num_days_from_monday();
        wd < 5
    });
    let mw = current_monthly_workdays(&cfg, &hol);
    calc::compute(&cfg, is_workday, mw, now)
}

/// 装载节假日数据并刷新托盘。
/// `persist=true` 表示数据来自网络，落本地缓存供下次启动直接用；
/// 内置兜底表不落盘——它只是「没有网络时的次优解」，不该污染缓存文件。
fn apply_holiday_cache(app: &tauri::AppHandle, mut c: holiday::HolidayCache, persist: bool) {
    if persist {
        c.fetched_at = Local::now().timestamp();
    }
    let state = app.state::<AppState>();
    {
        let mut hol = sync::lock(&state.holiday, "state.holiday");
        *hol = c;
        if persist {
            let snapshot = hol.clone();
            drop(hol);
            holiday::save_cache(&snapshot);
        }
    }
    let st = get_status(state.inner());
    tray::update_tray(app, &st);
}

/// 后台拉取并刷新节假日缓存；网络不可用时退回内置法定节假日表。
pub fn spawn_holiday_refresh(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let year = Local::now().year();
        match holiday::fetch_year(year).await {
            Ok(days) => {
                apply_holiday_cache(
                    &app,
                    holiday::HolidayCache {
                        year,
                        fetched_at: 0,
                        days,
                    },
                    true,
                );
            }
            Err(e) => {
                db::debug_log(&format!("节假日刷新失败: {}", e));
                match holiday::builtin_cache(year) {
                    Some(c) => {
                        db::debug_log(&format!("节假日：网络不可用，改用内置 {year} 年法定节假日表"));
                        apply_holiday_cache(&app, c, false);
                    }
                    None => db::debug_log(&format!(
                        "节假日：网络不可用且内置表未收录 {year} 年，退回周一至周五估算"
                    )),
                }
            }
        }
    });
}

#[tauri::command]
fn load_config(state: State<AppState>) -> config::Config {
    sync::lock(&state.config, "state.config").clone()
}

#[tauri::command]
fn save_config(state: State<AppState>, app: tauri::AppHandle, cfg: Value) {
    // 合并保存：以现有配置为基底，仅用前端传来的字段覆盖，保留前端未管理的字段
    // （如未来新增的后端字段），避免整份替换把未传字段重置成默认值。
    let existing = sync::lock(&state.config, "state.config").clone();
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
    *sync::lock(&state.config, "state.config") = merged.clone();
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
        let mut last = sync::lock(&state.last_date, "state.last_date");
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
    let cfg = sync::lock(&state.config, "state.config").clone();
    if !cfg.overtime_enabled {
        return;
    }
    let Some(lock_ts) = lock_monitor::last_lock_timestamp() else {
        return;
    };
    let mut seen = sync::lock(&state.last_lock_seen, "state.last_lock_seen");
    if *seen == Some(lock_ts) {
        return;
    }
    *seen = Some(lock_ts);
    drop(seen);

    let now = Local::now();
    let hol = sync::lock(&state.holiday, "state.holiday");
    let kind = overtime::day_kind(now.date_naive(), Some(&hol));
    // 非工作日只在开关打开时才算加班。此前这里直接 `if !is_workday { return }`，
    // 导致 weekend_overtime 开关形同虚设——开了也永远走不到计算。
    if kind.is_rest() && !cfg.weekend_overtime {
        return;
    }
    if let Some(lt) = Local.timestamp_opt(lock_ts, 0).single() {
        if let Some(record) = overtime::calc_record(now.date_naive(), lt, &cfg, Some(&hol)) {
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
    match holiday::fetch_year(year).await {
        Ok(days) => {
            let c = holiday::HolidayCache {
                year,
                fetched_at: 0,
                days,
            };
            let mw = current_monthly_workdays(&*sync::lock(&state.config, "state.config"), &c);
            apply_holiday_cache(&app, c, true);
            Ok(mw)
        }
        Err(e) => {
            // 手动刷新失败也让数字先准起来：内置表已收录的年份直接兜底，
            // 未收录才把错误抛给前端（此时确实给不出可信的工作日数）。
            db::debug_log(&format!("手动刷新节假日失败: {}", e));
            match holiday::builtin_cache(year) {
                Some(c) => {
                    db::debug_log(&format!("节假日：改用内置 {year} 年法定节假日表"));
                    let mw = current_monthly_workdays(&*sync::lock(&state.config, "state.config"), &c);
                    apply_holiday_cache(&app, c, false);
                    Ok(mw)
                }
                None => Err(format!(
                    "节假日数据获取失败，且内置表未收录 {year} 年：{}",
                    e
                )),
            }
        }
    }
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

/// 获取指定月份的加班记录（含预计算汇总字段）。
/// year / month 省略时取当前月——老调用方不传参也能正常工作。
#[tauri::command]
fn get_overtime_records(year: Option<i32>, month: Option<u32>) -> overtime::MonthlyOvertimeView {
    let now = Local::now();
    let y = year.unwrap_or_else(|| now.year());
    let m = month.unwrap_or_else(|| now.month());
    overtime::get_month(y, m).to_view(y, m)
}

/// 手动添加/修改某天加班记录（可补录历史月份，但不能是未来日期）。
/// 返回该记录**所属月份**的视图：补录 8 月时界面不会莫名跳回当月。
#[tauri::command]
fn save_overtime_record(
    state: State<AppState>,
    input: overtime::ManualOvertimeInput,
) -> Result<overtime::MonthlyOvertimeView, String> {
    let now = Local::now();
    let (y, m) = overtime::month_of(&input.date)
        .unwrap_or_else(|| (now.year(), now.month()));
    let cfg = sync::lock(&state.config, "state.config").clone();
    let hol = sync::lock(&state.holiday, "state.holiday").clone();
    overtime::save_manual(input, &cfg, Some(&hol))?;
    Ok(overtime::get_month(y, m).to_view(y, m))
}

/// 手动删除某天加班记录（历史月份同样可删）。返回该记录**所属月份**的视图。
#[tauri::command]
fn delete_overtime_record(
    date: String,
) -> Result<overtime::MonthlyOvertimeView, String> {
    let now = Local::now();
    let (y, m) = overtime::month_of(&date).unwrap_or_else(|| (now.year(), now.month()));
    overtime::delete_manual(&date)?;
    Ok(overtime::get_month(y, m).to_view(y, m))
}

/// 获取今日鼠标/键盘活动统计（逐小时 + 汇总 + 高频按键）
#[tauri::command]
fn get_activity_summary(date: Option<String>) -> activity::ActivitySummary {
    activity::summary_for(date.as_deref())
}

/// 获取今日应用使用时长统计（各应用累计 + 24 小时分布）
///
/// `known_icons`：前端已缓存图标的应用名，命中者不再回传 base64（图标是几 KB~几十 KB
/// 的 data URL，每 2 秒轮询整批搬运纯属浪费，只有新应用才需要传一次）。
#[tauri::command]
fn get_app_usage_summary(
    known_icons: Vec<String>,
    date: Option<String>,
) -> app_usage::AppUsageSummary {
    app_usage::summary(&known_icons, date.as_deref())
}

/// 获取今日媒体播放时长统计（各应用累计 + 24 小时分布）
#[tauri::command]
fn get_audio_usage_summary(
    known_icons: Vec<String>,
    date: Option<String>,
) -> audio_usage::AudioUsageSummary {
    audio_usage::summary(&known_icons, date.as_deref())
}

/// 前端调试日志落盘（写入 %APPDATA%/niuma-timer/debug.log，排查用户桌面环境用）
#[tauri::command]
fn write_debug_log(msg: String) {
    crate::db::debug_log(&msg);
}

/// 开机自启是否已开启。底层走 tauri-plugin-autostart（Windows 即 HKCU Run 键），
/// 用户在任务管理器里手工禁用后这里如实反映——注册表是唯一真相源。
#[tauri::command]
fn get_autostart(app: tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch()
        .is_enabled()
        .map_err(|e| format!("读取开机自启状态失败: {e:?}"))
}

/// 开启 / 关闭开机自启（经官方插件写 HKCU Run 键，无需管理员权限）。
/// 失败时把错误回给前端弹提示，避免开关显示成功、实际没写上。
#[tauri::command]
fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<String, String> {
    let mgr = app.autolaunch();
    if enabled {
        mgr.enable()
            .map(|_| "已开启开机自启".to_string())
            .map_err(|e| format!("开启开机自启失败: {e:?}"))
    } else {
        mgr.disable()
            .map(|_| "已关闭开机自启".to_string())
            .map_err(|e| format!("关闭开机自启失败: {e:?}"))
    }
}

/// 存储占用快照：设置页「数据存储」卡片展示用
#[tauri::command]
fn get_storage_info(state: State<'_, AppState>) -> Result<maintain::StorageInfo, String> {
    let cfg = sync::lock(&state.config, "state.config").clone();
    Ok(maintain::storage_info(&cfg))
}

/// 立即执行一次维护（WAL 收缩 + 过期图标 + 过期数据），返回执行后的占用快照。
/// 与调度器每日自动跑的是同一套逻辑，用户点按钮只是提前触发。
#[tauri::command]
fn run_maintenance(state: State<'_, AppState>) -> Result<maintain::StorageInfo, String> {
    let cfg = sync::lock(&state.config, "state.config").clone();
    maintain::run_daily(&cfg);
    Ok(maintain::storage_info(&cfg))
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
    },
    event: {
      // Tauri v2 的事件系统实际就是 plugin:event|listen 命令（见 @tauri-apps/api 的
      // _listen 实现），这里直接调它，省掉整个 @tauri-apps/api 依赖与打包链。
      // handler 收到的是完整 event 对象 { event, id, payload }。
      listen: function (evt, handler) {
        return window.__TAURI_INTERNALS__.invoke('plugin:event|listen', {
          event: evt,
          target: { kind: 'Any' },
          handler: window.__TAURI_INTERNALS__.transformCallback(handler)
        });
      }
    }
  };
}
"#;

/// 修复任务栏图标模糊：
/// tauri-codegen 生成默认窗口图标时只解码 icon.ico 的**第一个图层**（本项目为 16×16），


fn main() {
    install_crash_log();
    let app = tauri::Builder::default()
        // 单例模式：若已有实例在运行，第二个实例启动时被拦截，
        // 并在回调里把已存在的主窗口显示并置前，自己退出。
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec![])))
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
            // SQLite 首次调用 conn() 时建目录 + 开库 + 建表（WAL）。
            // 程序未发布、无旧库旧 JSON，不做任何迁移；schema 变更直接改 DDL 重建库。
            db::conn();
            trace_startup("setup: db ready");

            // 载入本地节假日缓存；无缓存时用内置法定节假日表兜底，
            // 避免首屏就按「周一至周五」估算（2026-10 会算成 22 天，实际 18 天）
            {
                let year = Local::now().year();
                if let Some(c) = holiday::load_cache(year).or_else(|| holiday::builtin_cache(year))
                {
                    *sync::lock(&app.state::<AppState>().holiday, "state.holiday") = c;
                } else {
                    db::debug_log(&format!("节假日：无缓存且内置表未收录 {year} 年，退回周一至周五估算"));
                }
            }
            trace_startup("setup: holiday cache loaded");
            // 修复任务栏图标模糊：用 exe 内嵌多尺寸 ico 按 DPI 重新设置窗口图标
            #[cfg(windows)]
            if let Some(w) = app.get_webview_window("main") {
                if let Ok(hwnd) = w.hwnd() {
                    crate::win::set_window_icons_from_resource(hwnd);
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
            apply_monitor_switches(&sync::lock(&app.state::<AppState>().config, "state.config").clone());
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

            // 统一周期调度：托盘 UI 刷新(1s) / 跨天+加班检测(5s) / 活动落盘与
            // 应用结算(10s) 合并成一条「1 秒一拍」的线程——它们都是纯周期任务，
            // 原先各占一个常驻线程纯属浪费。带 Win32 消息循环 / COM 亲和的采集
            // 线程不在此列，仍在各自模块里（详见 scheduler 模块文档）。
            // 必须在 activity::start() / app_usage::start() 之后：采集线程先就位。
            scheduler::start(apph);
            trace_startup("setup: scheduler started");
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
            write_debug_log,
            get_autostart,
            set_autostart,
            get_storage_info,
            run_maintenance
        ])
        .build(tauri::generate_context!())
    ;
    match app {
        Ok(app) => {
            trace_startup("app: built, entering run loop");
            app.run(|_app_handle, event| {
                // 程序退出前：把鼠标/键盘统计与应用使用时长的最后增量落盘，重启后不丢数据
                if let tauri::RunEvent::Exit = event {
                    activity::shutdown();
                    app_usage::shutdown();
                    audio_usage::shutdown();
                }
            });
        }
        Err(e) => {
            // build 失败（最常见：目标机缺 WebView2 运行时）→ 弹窗告知具体原因，
            // 不再静默退出让用户误以为「双击无反应」。
            let detail = e.to_string();
            let hint = if detail.to_lowercase().contains("webview") {
                "\n\n推测原因：系统缺少 WebView2 运行时（Win11 通常自带；干净/企业精简镜像可能未预装）。\n请到微软官网下载安装「WebView2 Runtime (Evergreen Bootstrapper)」后重试。"
            } else {
                ""
            };
            let msg = format!("牛马计时器启动失败：\n{detail}{hint}");
            show_fatal(&msg);
            std::process::exit(1);
        }
    }
    // 前端资源自动重编：build.rs 用 frontend 目录内容指纹注入
    // `cargo:rustc-env=TAURI_FRONTEND_FP`，内容一变即触发本 crate 重编、
    // generate_context! 重跑并重新嵌入最新 HTML/CSS/JS，无需手动改动任何标记。
}
