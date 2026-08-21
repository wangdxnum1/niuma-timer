#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod calc;
mod config;
mod holiday;
mod icon_render;
mod lock_monitor;
mod overtime;
mod tray;

use std::sync::Mutex;

use chrono::{Datelike, Local, NaiveDate, TimeZone};
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
fn save_config(state: State<AppState>, app: tauri::AppHandle, cfg: config::Config) {
    *state.config.lock().unwrap() = cfg.clone();
    config::save(&cfg);
    let st = get_status(state.inner());
    tray::update_tray(&app, &st);
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

/// 显示并设置焦点到主窗口
#[tauri::command]
fn show_window(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// 仅把主窗口提到前台
#[tauri::command]
fn focus_window(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
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

fn main() {
    tauri::Builder::default()
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
            // 载入本地节假日缓存
            {
                let year = Local::now().year();
                if let Some(c) = holiday::load_cache(year) {
                    *app.state::<AppState>().holiday.lock().unwrap() = c;
                }
            }
            // 创建托盘
            let _tray = tray::create_tray(app)?;

            // 跨月自动归档：把历史月数据拆成 overtime-YYYY-MM.json 独立文件
            overtime::ensure_archived();

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

            // 每秒刷新一轮
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                let state = apph.state::<AppState>();
                let today = Local::now().date_naive();
                {
                    let mut last = state.last_date.lock().unwrap();
                    if *last != today {
                        *last = today;
                        drop(last);
                        // 跨天：重新拉取节假日
                        spawn_holiday_refresh(apph.clone());
                        // 跨天也可能跨月：把已结束的月份归档为独立文件
                        overtime::ensure_archived();
                    }
                }
                let st = get_status(state.inner());
                tray::update_tray(&apph, &st);

                // ---- 加班锁屏检测 ----
                let cfg = state.config.lock().unwrap().clone();
                if cfg.overtime_enabled {
                    if let Some(lock_ts) = lock_monitor::last_lock_timestamp() {
                        let mut seen = state.last_lock_seen.lock().unwrap();
                        if *seen != Some(lock_ts) {
                            *seen = Some(lock_ts);
                            drop(seen);

                            let now = Local::now();
                            let is_workday = state
                                .holiday
                                .lock()
                                .unwrap()
                                .is_workday(now.date_naive())
                                .unwrap_or_else(|| {
                                    now.weekday().num_days_from_monday() < 5
                                });

                            if is_workday {
                                if let Some(lt) = Local.timestamp_opt(lock_ts, 0).single() {
                                    if let Some(record) =
                                        overtime::calc_record(now.date_naive(), lt, &cfg)
                                    {
                                        overtime::upsert_record(record);
                                    }
                                }
                            }
                        }
                    }
                }
            });
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
            delete_overtime_record
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
