#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod calc;
mod config;
mod holiday;
mod icon_render;
mod tray;

use std::sync::Mutex;

use chrono::{Datelike, Local, NaiveDate};
use tauri::{Manager, State};

struct AppState {
    config: Mutex<config::Config>,
    holiday: Mutex<holiday::HolidayCache>,
    last_date: Mutex<NaiveDate>,
}

impl Default for AppState {
    fn default() -> Self {
        AppState {
            config: Mutex::new(config::load()),
            holiday: Mutex::new(holiday::HolidayCache::default()),
            last_date: Mutex::new(Local::now().date_naive()),
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
fn get_status(state: &AppState) -> calc::DayStatus {
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

fn main() {
    tauri::Builder::default()
        .enable_global_tauri()
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

            // 窗口关闭仅隐藏，不退出程序
            if let Some(w) = app.get_webview_window("main") {
                w.on_window_event(move |event| {
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
                    }
                }
                let st = get_status(state.inner());
                tray::update_tray(&apph, &st);
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_config,
            save_config,
            refresh_holidays,
            get_status_cmd
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
