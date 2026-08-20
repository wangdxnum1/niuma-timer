use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::Manager;
use tauri::menu::{Menu, MenuId, MenuItem};
use tauri::tray::{MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, App, Emitter, Listener, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

use crate::calc::DayStatus;
use crate::icon_render::static_icon;

/// 彩色悬停卡片尺寸（逻辑像素）
const HOVER_CARD_W: f64 = 320.0;
// 高度留足：内容（大金额+4宫格+状态行）在默认行高下约 170px，
// 210 会把底部状态行挤出圆角框（overflow 溢出到透明背景），故加到 220
const HOVER_CARD_H: f64 = 220.0;

/// 是否处于"待隐藏"状态：鼠标已离开托盘、淡出动画进行中。
/// 用于防止"兜底隐藏线程"误关掉用户重新进入时正在淡入的卡片。
static HIDE_PENDING: AtomicBool = AtomicBool::new(false);

/// hover_card 窗口懒创建锁：防止托盘事件并发触发重复创建
static HOVER_CREATING: AtomicBool = AtomicBool::new(false);

/// 最近一次悬停位置缓存：hover_card 页面首次加载完成（hover_ready）后，
/// 若鼠标仍停留在托盘上，用它补一次定位 + 淡入，避免"首次悬停事件丢失"。
static HOVER_PENDING_POS: Mutex<Option<PhysicalPosition<f64>>> = Mutex::new(None);

/// 看门狗线程是否已在运行（防重复启动）。
/// 背景：Windows 托盘 Leave 事件偶发丢失（鼠标快速滑出时），
/// 一旦丢失卡片会一直残留。因此由看门狗每 150ms 轮询鼠标位置兜底。
static WATCHDOG_RUNNING: AtomicBool = AtomicBool::new(false);

/// 悬停卡片当前矩形（屏幕物理坐标 x,y,w,h）：用于判断鼠标是否停留在卡片上，
/// 避免"鼠标从托盘滑入卡片"时因已离开托盘图标而被误隐藏（闪烁）。
/// 卡片显示时写入，隐藏时清空。
static HOVER_CARD_RECT: Mutex<Option<(f64, f64, f64, f64)>> = Mutex::new(None);

/// 悬停卡片延迟显示时长（毫秒）：鼠标进入托盘后等待该时长，
/// 若仍停留在托盘范围内才显示，模仿系统原生 tooltip 的延迟出现效果。
const HOVER_SHOW_DELAY_MS: u64 = 400;

/// 延迟显示是否待执行：进入托盘后置 true，离开或被取消后置 false。
/// 延迟计时器据此判断是否还应弹出卡片。
static SHOW_PENDING: AtomicBool = AtomicBool::new(false);

/// 延迟计时器是否已在运行（防重复启动）：一个计时期内只跑一个线程。
static SHOW_TIMER_ACTIVE: AtomicBool = AtomicBool::new(false);

/// 悬停卡片调试日志：写入临时目录（Windows GUI 程序无控制台，eprintln 不可见）。
/// 路径：%TEMP%\niuma_timer_hover.log
fn hover_log(msg: &str) {
    use std::io::Write;
    let p = std::env::temp_dir().join("niuma_timer_hover.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{msg}");
    }
}

/// 托盘图标中心锚点缓存：rect() 成功时刷新。即便 rect() 后续偶发失败，
/// 也用缓存中心定位卡片，避免卡片跟随实时光标左右平移（"抖动"的主因）。
static TRAY_CENTER: Mutex<Option<PhysicalPosition<f64>>> = Mutex::new(None);

/// 计算托盘图标中心锚点：卡片水平居中于此（精确居中，不受鼠标进入方向影响）。
/// Windows 托盘图标很小，rect() 拿不到时退回缓存中心，再不行才退光标位置。
fn tray_anchor(tray: &TrayIcon, fallback: PhysicalPosition<f64>) -> PhysicalPosition<f64> {
    if let Ok(Some(r)) = tray.rect() {
        // tauri 的 Position/Size 是枚举（Physical/Logical），需解包后求中心
        let (cx, cy) = match (r.position, r.size) {
            (tauri::Position::Physical(p), tauri::Size::Physical(s)) => (
                p.x as f64 + s.width as f64 / 2.0,
                p.y as f64 + s.height as f64 / 2.0,
            ),
            (tauri::Position::Logical(p), tauri::Size::Logical(s)) => (
                p.x + s.width / 2.0,
                p.y + s.height / 2.0,
            ),
            // 混合量纲（罕见）：各自按数值相加即可，误差一个像素量级
            (tauri::Position::Physical(p), tauri::Size::Logical(s)) => (
                p.x as f64 + s.width / 2.0,
                p.y as f64 + s.height / 2.0,
            ),
            (tauri::Position::Logical(p), tauri::Size::Physical(s)) => (
                p.x + s.width as f64 / 2.0,
                p.y + s.height as f64 / 2.0,
            ),
        };
        let c = PhysicalPosition::new(cx, cy);
        // 刷新缓存，供 rect() 偶发失败时仍能稳定定位
        *TRAY_CENTER.lock().unwrap() = Some(c);
        c
    } else if let Some(c) = *TRAY_CENTER.lock().unwrap() {
        // rect 拿不到：优先用缓存中心（托盘位置基本不变），避免卡片跟手抖动
        c
    } else {
        fallback
    }
}

/// 创建托盘图标、菜单（hover_card 窗口改为首次悬停时懒创建，
/// 启动路径不再创建额外 WebView2 窗口，杜绝阻塞主窗口渲染）
pub fn create_tray(app: &App) -> tauri::Result<TrayIcon> {
    let settings = MenuItem::with_id(app, "settings", "主界面", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "刷新工作日", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&settings, &refresh, &quit])?;

    // 页面就绪握手：hover_card.html 加载完成后 emit "hover_ready"，
    // 若鼠标仍悬停在托盘上，则补一次定位与淡入（页面加载耗时不定，
    // 不依赖事件恰好送达的时序，也不用 visibilitychange 猜测窗口状态）
    let apph = app.handle().clone();
    let _ = app.listen("hover_ready", move |_event| {
        hover_log("[hover_card] 页面就绪 (hover_ready)");
        let pos = *HOVER_PENDING_POS.lock().unwrap();
        if let Some(pos) = pos {
            position_hover_card(&apph, pos);
        }
    });
    // 前端脚本错误上报（hover_card.html 的 window.onerror/unhandledrejection）
    let _ = app.listen("hover_error", |e| {
        let msg = e.payload();
        hover_log(&format!("[hover_card] 前端错误: {msg}"));
    });

    let icon = static_icon();
    let tray = TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("牛马计时器启动中…")
        .menu(&menu)
        // Windows 平台默认左键点击也弹菜单（show_menu_on_left_click=true），
        // 导致左键双击直接弹出右键菜单而非打开主界面。显式关闭后，
        // 左键只产生 Click/DoubleClick 事件，右键仍正常弹菜单
        .show_menu_on_left_click(false)
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
            // 进入托盘：延迟一段时间后（鼠标仍停留）才显示卡片，
            // 模仿系统原生 tooltip 的延迟出现行为，避免划过托盘就弹窗
            TrayIconEvent::Enter { position, .. } => {
                hover_log(&format!(
                    "[hover_card] Enter 托盘 pos=({:.0},{:.0})",
                    position.x, position.y
                ));
                let apph = tray.app_handle().clone();
                // 记录进入位置（延迟计时器到点时若鼠标仍在，用它定位）
                *HOVER_PENDING_POS.lock().unwrap() = Some(tray_anchor(tray, position));
                SHOW_PENDING.store(true, Ordering::Relaxed);
                // 仅启动一个延迟计时器，避免 Move/Enter 高频事件反复起线程
                if !SHOW_TIMER_ACTIVE.swap(true, Ordering::SeqCst) {
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(HOVER_SHOW_DELAY_MS));
                        let app2 = apph.clone();
                        // 到点时校验：仍待显示、且鼠标停留在托盘范围内才弹出
                        if SHOW_PENDING.load(Ordering::Relaxed) {
                            let cur = app2.cursor_position().unwrap_or_default();
                            if mouse_near_tray(&app2, cur) {
                                let anchor =
                                    HOVER_PENDING_POS.lock().unwrap().unwrap_or(cur);
                                position_hover_card(&app2, anchor);
                            }
                        }
                        SHOW_TIMER_ACTIVE.store(false, Ordering::Relaxed);
                    });
                }
            }
            // 鼠标在托盘内移动：卡片已可见时跟随移动（保持原位、避免抖动）；
            // 未显示期间只更新待显位置，真正的显示由延迟计时器统一处理
            TrayIconEvent::Move { position, .. } => {
                let app = tray.app_handle();
                if let Some(w) = app.get_webview_window("hover_card") {
                    if w.is_visible().unwrap_or(false) {
                        let anchor = tray_anchor(tray, position);
                        position_hover_card(app, anchor);
                    } else {
                        *HOVER_PENDING_POS.lock().unwrap() =
                            Some(tray_anchor(tray, position));
                    }
                }
            }
            TrayIconEvent::Leave { .. } => {
                hover_log("[hover_card] Leave 托盘");
                // 取消待显示的延迟任务，避免离开后卡片又弹出。
                // 此处【不】立即隐藏：Windows 托盘边缘偶发 Enter/Leave 高频抖动
                // （左右滑出时尤甚），若 Leave 立即隐藏、紧接着边缘回弹的 Enter
                // 又触发显示，会出现闪烁。统一交给看门狗（带去抖）裁决隐藏。
                SHOW_PENDING.store(false, Ordering::Relaxed);
            }
            // 左键双击：收起悬停卡片并弹出主界面
            // （show_menu_on_left_click(false) 后左键不再弹菜单，事件可正常收到）
            TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => {
                hover_log("[tray] 左键双击 → 打开主界面");
                hide_hover_card(tray.app_handle());
                if let Some(w) = tray.app_handle().get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                    // 延迟再抢一次焦点：绕过 Windows 前台锁定（SetForegroundWindow
                    // 对后台进程首次调用可能被拒绝，焦点最终仍落在托盘/explorer）
                    let w2 = w.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(300));
                        let _ = w2.set_focus();
                    });
                }
            }
            _ => {}
        })
        .build(app)?;
    Ok(tray)
}

/// 懒创建 hover_card 窗口（首次悬停托盘时才创建，且发生在托盘事件线程，
/// 不影响主窗口启动渲染；创建失败仅禁用悬停卡片，不影响主程序）
fn ensure_hover_card(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    if let Some(w) = app.get_webview_window("hover_card") {
        return Some(w);
    }
    // 并发保护：托盘事件可能同时触发（Enter/Move），只允许一个创建流程
    if HOVER_CREATING.swap(true, Ordering::SeqCst) {
        return None;
    }
    let created = WebviewWindowBuilder::new(
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
    HOVER_CREATING.store(false, Ordering::SeqCst);
    match created {
        Ok(w) => {
            hover_log("[hover_card] 窗口已创建");
            // 鼠标穿透：卡片不拦截任何点击，悬停托盘区域不受影响
            let _ = w.set_ignore_cursor_events(true);
            Some(w)
        }
        Err(e) => {
            hover_log(&format!("[hover_card] 窗口创建失败: {e}"));
            None
        }
    }
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
    // 记录悬停位置（供页面就绪后的 hover_ready 补显）
    *HOVER_PENDING_POS.lock().unwrap() = Some(pos);

    let Some(w) = ensure_hover_card(app) else {
        return;
    };
    // 用户已重新进入托盘，取消待隐藏状态，避免兜底线程误关卡片
    HIDE_PENDING.store(false, Ordering::Relaxed);

    // 卡片水平居中于托盘图标正上方（x 对齐锚点中心，y 在锚点上方留间隙）。
    // 参考：Windows 11 系统原生 tooltip 底部距任务栏约 10px，
    // 这里留 24px，比系统默认再高一点，视觉更透气
    let mut x = pos.x - HOVER_CARD_W / 2.0;
    let mut y = pos.y - HOVER_CARD_H - 24.0;

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

    // 记录卡片当前屏幕矩形（供"鼠标是否仍在卡片上"判断，防移入卡片时闪烁）
    *HOVER_CARD_RECT.lock().unwrap() = Some((x, y, HOVER_CARD_W, HOVER_CARD_H));

    let was_visible = w.is_visible().unwrap_or(false);
    let _ = w.set_position(PhysicalPosition::new(x, y));
    if !was_visible {
        hover_log(&format!("[hover_card] show() pos=({x:.0},{y:.0})"));
        // show() 不抢焦点（Windows SW_SHOW），避免干扰用户操作
        let _ = w.show();
        // 启动看门狗：Leave 事件偶发丢失时轮询鼠标位置兜底隐藏
        start_hover_watchdog(app);
    }
    // 立即推送一帧数据（此后由 update_tray 每秒续推）
    let st = crate::get_status(app.state::<crate::AppState>().inner());
    let _ = w.emit("hover_data", st);
    // 淡入事件：立即 + 分档重试，覆盖页面首次加载期（JS 端幂等，重复无副作用）；
    // 页面加载超过重试窗口时，由 hover_ready 握手兜底补显
    let w2 = w.clone();
    std::thread::spawn(move || {
        // 分档重试覆盖 WebView2 首次冷启动加载期（可达 1s+）；
        // 超过窗口期仍未就绪时，由 hover_ready 握手 + 前端 visibilitychange 自治兜底
        for (idx, delay) in [0u64, 200, 500, 1000, 1800].iter().enumerate() {
            if HIDE_PENDING.load(Ordering::Relaxed) {
                break;
            }
            if *delay > 0 {
                std::thread::sleep(Duration::from_millis(*delay));
            }
            let _ = w2.emit("hover_show", ());
            let _ = idx; // 保留 enumerate 以备调试
        }
    });
}

/// 鼠标离开托盘：播放淡出动画，动画结束后由页面自行隐藏窗口
fn hide_hover_card(app: &AppHandle) {
    // 卡片即将隐藏，清除其矩形记录（下次显示时由 position_hover_card 重新写入）
    HOVER_CARD_RECT.lock().unwrap().take();
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

/// 启动悬停卡片看门狗（常驻线程，仅首次调用时启动一次）。
/// 每 150ms 检查一次：卡片可见但鼠标已离开"托盘图标 + 卡片"活动区域 → 强制隐藏。
/// 这解决 Windows 托盘 Leave 事件偶发丢失导致的"卡片残留"，并通过连续 2 次
/// （约 300ms）确认离开的去抖逻辑，过滤边缘 Enter/Leave 高频抖动造成的闪烁。
fn start_hover_watchdog(app: &AppHandle) {
    if WATCHDOG_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app2 = app.clone();
    std::thread::spawn(move || {
        hover_log("[hover_card] 看门狗已启动");
        let mut gone_polls: u32 = 0;
        loop {
            std::thread::sleep(Duration::from_millis(150));
            let Some(w) = app2.get_webview_window("hover_card") else {
                gone_polls = 0;
                continue;
            };
            // 卡片不可见 → 无需检查
            if !w.is_visible().unwrap_or(false) {
                gone_polls = 0;
                continue;
            }
            match app2.cursor_position() {
                Ok(pos) => {
                    if mouse_in_active_region(&app2, pos) {
                        // 仍在活动区域（托盘图标内 或 悬停卡片上）→ 保持显示
                        gone_polls = 0;
                    } else {
                        // 离开活动区域：需连续 2 次轮询（约 300ms）确认才隐藏，
                        // 过滤边缘抖动 / 瞬时误判，避免卡片闪烁
                        gone_polls += 1;
                        if gone_polls >= 2 {
                            hover_log(&format!(
                                "[hover_card] 看门狗: 鼠标持续移出 ({:.0},{:.0})，强制隐藏",
                                pos.x, pos.y
                            ));
                            hide_hover_card(&app2);
                            gone_polls = 0;
                        }
                    }
                }
                Err(_) => {
                    // 取不到光标位置时不动作，避免误隐藏
                    gone_polls = 0;
                }
            }
        }
    });
}

/// 鼠标是否在"保持卡片可见"的活动区域内：托盘图标范围内，或悬停卡片矩形内
/// （均带余量）。用于看门狗与 Leave 判断，避免鼠标移入卡片上时被误隐藏（闪烁）。
fn mouse_in_active_region(app: &AppHandle, pos: PhysicalPosition<f64>) -> bool {
    if mouse_near_tray(app, pos) {
        return true;
    }
    if let Some((cx, cy, cw, ch)) = *HOVER_CARD_RECT.lock().unwrap() {
        const PAD: f64 = 12.0;
        return pos.x >= cx - PAD
            && pos.x <= cx + cw + PAD
            && pos.y >= cy - PAD
            && pos.y <= cy + ch + PAD;
    }
    false
}

/// 鼠标是否仍在托盘图标范围内（带 12px 余量防边缘抖动）。
/// 图标 rect 拿不到时退回悬停锚点 ±40px 假设范围；完全未知时返回 true（不误隐藏）。
fn mouse_near_tray(app: &AppHandle, pos: PhysicalPosition<f64>) -> bool {
    const PAD: f64 = 12.0;
    if let Some(tray) = app.tray_by_id("main") {
        if let Ok(Some(r)) = tray.rect() {
            let (rx, ry, rw, rh) = match (r.position, r.size) {
                (tauri::Position::Physical(p), tauri::Size::Physical(s)) => {
                    (p.x as f64, p.y as f64, s.width as f64, s.height as f64)
                }
                (tauri::Position::Logical(p), tauri::Size::Logical(s)) => {
                    (p.x, p.y, s.width, s.height)
                }
                (tauri::Position::Physical(p), tauri::Size::Logical(s)) => {
                    (p.x as f64, p.y as f64, s.width, s.height)
                }
                (tauri::Position::Logical(p), tauri::Size::Physical(s)) => {
                    (p.x, p.y, s.width as f64, s.height as f64)
                }
            };
            return pos.x >= rx - PAD
                && pos.x <= rx + rw + PAD
                && pos.y >= ry - PAD
                && pos.y <= ry + rh + PAD;
        }
    }
    if let Some(a) = *HOVER_PENDING_POS.lock().unwrap() {
        return (pos.x - a.x).abs() <= 40.0 && (pos.y - a.y).abs() <= 40.0;
    }
    true
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
