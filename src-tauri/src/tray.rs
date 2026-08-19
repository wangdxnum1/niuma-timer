use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::Manager;
use tauri::menu::{Menu, MenuId, MenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, App, Emitter, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

use crate::calc::DayStatus;
use crate::icon_render::static_icon;

/// 彩色悬停卡片尺寸（逻辑像素）
const HOVER_CARD_W: f64 = 320.0;
const HOVER_CARD_H: f64 = 210.0;

/// 是否处于"待隐藏"状态：鼠标已离开托盘、淡出动画进行中。
/// 用于防止"兜底隐藏线程"误关掉用户重新进入时正在淡入的卡片。
static HIDE_PENDING: AtomicBool = AtomicBool::new(false);

/// 创建托盘图标、菜单与彩色悬停卡片窗口
pub fn create_tray(app: &App) -> tauri::Result<TrayIcon> {
    let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "刷新工作日", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&settings, &refresh, &quit])?;

    // 彩色悬停卡片窗口（初始隐藏；仅当配置开启时才会被托盘事件唤起）
    // 页面：frontend/hover_card.html，数据由 Rust 端 emit "hover_data" 推送
    let _ = WebviewWindowBuilder::new(
        app,
        "hover_card",
        WebviewUrl::App("hover_card.html".into()),
    )
    .title("牛马计时器 · 悬停卡片")
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .shadow(false)
    .inner_size(HOVER_CARD_W, HOVER_CARD_H)
    .resizable(false)
    .visible(false)
    .build();
    if let Some(w) = app.get_webview_window("hover_card") {
        // 鼠标穿透：卡片不拦截任何点击，悬停托盘区域不受影响
        let _ = w.set_ignore_cursor_events(true);
    }

    let icon = static_icon();
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
        .on_tray_icon_event(|tray, event| match event {
            // 进入托盘：定位并显示卡片
            TrayIconEvent::Enter { position, .. } => {
                position_hover_card(tray.app_handle(), position);
            }
            // 鼠标在托盘内移动：卡片已可见时保持原位，避免抖动；
            // 仅当卡片尚未显示（首次进入、动画未就绪）时补一次定位
            TrayIconEvent::Move { position, .. } => {
                let app = tray.app_handle();
                if let Some(w) = app.get_webview_window("hover_card") {
                    if !w.is_visible().unwrap_or(false) {
                        position_hover_card(app, position);
                    }
                }
            }
            TrayIconEvent::Leave { .. } => {
                hide_hover_card(tray.app_handle());
            }
            _ => {}
        })
        .build(app)?;
    Ok(tray)
}

/// 托盘悬停：显示/移动彩色卡片到鼠标旁
fn position_hover_card(app: &AppHandle, pos: PhysicalPosition<f64>) {
    let on = app
        .state::<crate::AppState>()
        .config
        .lock()
        .unwrap()
        .tray_hover_card;
    if !on {
        return;
    }
    let Some(w) = app.get_webview_window("hover_card") else {
        return;
    };
    // 用户已重新进入托盘，取消待隐藏状态，避免兜底线程误关卡片
    HIDE_PENDING.store(false, Ordering::Relaxed);

    // 默认放到鼠标左上方（托盘通常在屏幕底部）
    let mut x = pos.x - HOVER_CARD_W + 24.0;
    let mut y = pos.y - HOVER_CARD_H - 14.0;

    // 防溢出屏幕（按主屏粗略钳制；任务栏在顶部时改为鼠标下方）
    if let Ok(Some(m)) = app.primary_monitor() {
        let size = m.size();
        let sw = size.width as f64;
        let sh = size.height as f64;
        if y < 8.0 {
            y = pos.y + 18.0;
        }
        if x < 8.0 {
            x = 8.0;
        }
        if x + HOVER_CARD_W > sw - 8.0 {
            x = sw - HOVER_CARD_W - 8.0;
        }
        if y + HOVER_CARD_H > sh - 8.0 {
            y = sh - HOVER_CARD_H - 8.0;
        }
        if y < 8.0 {
            y = 8.0;
        }
    }

    let was_visible = w.is_visible().unwrap_or(false);
    let _ = w.set_position(PhysicalPosition::new(x, y));
    if !was_visible {
        // show() 不抢焦点（Windows SW_SHOW），避免干扰用户操作
        let _ = w.show();
        // 页面若尚未加载完成，本次 hover_show 事件可能丢失：
        // 延迟补发一次，保证首次悬停也能平滑淡入（JS 端幂等，重复收到无副作用）
        let w2 = w.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            if HIDE_PENDING.load(Ordering::Relaxed) {
                return;
            }
            let _ = w2.emit("hover_show", ());
        });
    } else {
        // 窗口已可见：可能正处于淡出中，立即恢复淡入（JS 端幂等）
        let _ = w.emit("hover_show", ());
    }
    // 立即推送一帧数据（此后由 update_tray 每秒续推）
    let st = crate::get_status(app.state::<crate::AppState>().inner());
    let _ = w.emit("hover_data", st);
}

/// 鼠标离开托盘：播放淡出动画，动画结束后由页面自行隐藏窗口
fn hide_hover_card(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("hover_card") {
        if w.is_visible().unwrap_or(false) {
            HIDE_PENDING.store(true, Ordering::Relaxed);
            let _ = w.emit("hover_hide", ());
            // 兜底：页面动画异常（如 transition 未触发）时强制隐藏；
            // 若期间用户重新进入托盘，HIDE_PENDING 被重置，不会误关
            let app2 = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(450));
                if HIDE_PENDING.load(Ordering::Relaxed) {
                    if let Some(w) = app2.get_webview_window("hover_card") {
                        let _ = w.hide();
                    }
                    HIDE_PENDING.store(false, Ordering::Relaxed);
                }
            });
        }
    }
}

/// 更新托盘的 tooltip / 悬停卡片
/// - 彩色卡片开启：清空系统 tooltip（避免双显），卡片可见时每秒推送实时数据
/// - 彩色卡片关闭：恢复系统原生 tooltip
pub fn update_tray(app: &AppHandle, status: &DayStatus) {
    let cfg = app.state::<crate::AppState>().config.lock().unwrap().clone();
    if let Some(tray) = app.tray_by_id("main") {
        if cfg.tray_hover_card {
            let _ = tray.set_tooltip::<&str>(None);
        } else {
            let _ = tray.set_tooltip(Some(&status.tooltip));
        }
    }
    if let Some(w) = app.get_webview_window("hover_card") {
        let visible = w.is_visible().unwrap_or(false);
        if visible && cfg.tray_hover_card {
            let _ = w.emit("hover_data", status.clone());
        } else if visible && !cfg.tray_hover_card {
            // 关闭开关时立即收回卡片
            let _ = w.hide();
        }
    }
}
