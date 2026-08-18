use tauri::Manager;
use tauri::menu::{Menu, MenuId, MenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, App};

use crate::calc::DayStatus;
use crate::icon_render::render_icon;

/// 创建托盘图标与菜单
pub fn create_tray(app: &App) -> tauri::Result<TrayIcon> {
    let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "刷新工作日", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&settings, &refresh, &quit])?;

    let icon = render_icon("¥0");
    let tray = TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("牛马计时器启动中…")
        .menu(&menu)
        .on_menu_event(|app, event| {
            if event.id == MenuId::new("settings") {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            } else if event.id == MenuId::new("refresh") {
                crate::spawn_holiday_refresh(app.clone());
            } else if event.id == MenuId::new("quit") {
                app.exit(0);
            }
        })
        .build(app)?;
    Ok(tray)
}

/// 更新托盘的 tooltip 与动态图标
pub fn update_tray(app: &AppHandle, status: &DayStatus) {
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(&status.tooltip));
        let _ = tray.set_icon(Some(render_icon(&status.icon_text)));
    }
}
