//! 托盘悬停卡片状态机（P3 重写）
//!
//! 旧实现用 9 个全局 static + 看门狗线程 + 延迟计时器 + 点击冷却 + 分档重发，
//! 多个线程并发读写同一批状态，难以推理。本版本改为 **actor 模型**：
//! - 托盘事件 / hover_ready 握手只向通道发送消息，不做任何状态变更；
//! - 单个工作线程独占 `HoverController`（持有全部状态），用 `recv_timeout`
//!   统一驱动延迟显示、真实光标/菜单状态采样与首次加载重发；
//! - 空闲时每 100ms 采样兜底（无忙循环）；状态变更只发生在一个线程。
//!
//! 前端握手协议（hover_ready / hover_show / hover_hide / hover_data）保持不变。

use crate::sync;
mod hover_state;
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::menu::{Menu, MenuId, MenuItem};
use tauri::tray::{MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::Emitter;
use tauri::Listener;
use tauri::Manager;
use tauri::{App, AppHandle, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::calc::DayStatus;
use crate::icon_render::static_icon;

/// 彩色悬停卡片尺寸（逻辑像素）。高度 352 = 卡片 336 + 上下各 8px 透明边距。
///
/// **必须与 `frontend/hover_card.html` 的 `body { height }` 保持一致**：
/// `body height == HOVER_CARD_H`（body 自带上下各 8px padding，卡片实际高 = 336）。
/// 卡片内部是 flex 列布局，窗口一矮，
/// 唯一可收缩的 `.hero`（唯一带 overflow:hidden 的子项，自动最小尺寸为 0）
/// 就会被压扁、34px 大字金额被裁没。2026-09-14 实际踩过这个坑，
/// `scripts/test_hover_card.js` 已加两侧尺寸一致性断言防回归。
const HOVER_CARD_W: f64 = 380.0;
const HOVER_CARD_H: f64 = 352.0;

#[cfg(test)]
mod placement_tests {
    use super::*;

    #[test]
    fn scaled_card_stays_above_bottom_taskbar() {
        for scale in [1.0, 1.25, 1.5, 2.0, 2.5] {
            let area = (0.0, 0.0, 2880.0, 1704.0);
            let (x, y, w, h) = hover_card_rect(PhysicalPosition::new(2730.0, 1752.0), scale, area);
            assert_eq!((w, h), ((380.0 * scale).round(), (352.0 * scale).round()));
            assert!(x >= 8.0 * scale && x + w <= area.2 - 8.0 * scale);
            assert!(
                y + h <= area.3 - 8.0 * scale,
                "scale={scale}: card bottom {} crosses work area",
                y + h
            );
        }
    }

    #[test]
    fn placement_uses_secondary_monitor_origin() {
        for area in [
            (-2560.0, -200.0, 2560.0, 1392.0),
            (2880.0, 400.0, 1920.0, 1032.0),
        ] {
            let anchor = PhysicalPosition::new(area.0 + area.2 - 60.0, area.1 + area.3 + 24.0);
            let (x, y, w, h) = hover_card_rect(anchor, 1.5, area);
            assert!(x >= area.0 && x + w <= area.0 + area.2);
            assert!(y >= area.1 && y + h <= area.1 + area.3);
        }
    }

    #[test]
    fn top_and_side_taskbars_stay_outside_card() {
        for (anchor, area) in [
            (
                PhysicalPosition::new(1600.0, 30.0),
                (0.0, 60.0, 1920.0, 1020.0),
            ),
            (
                PhysicalPosition::new(30.0, 900.0),
                (60.0, 0.0, 1860.0, 1080.0),
            ),
            (
                PhysicalPosition::new(1890.0, 900.0),
                (0.0, 0.0, 1860.0, 1080.0),
            ),
        ] {
            let (x, y, w, h) = hover_card_rect(anchor, 1.5, area);
            assert!(x >= area.0 && x + w <= area.0 + area.2);
            assert!(y >= area.1 && y + h <= area.1 + area.3);
        }
    }

    #[test]
    fn monitor_switch_recalculates_physical_size() {
        let external = hover_card_rect(
            PhysicalPosition::new(-100.0, 1050.0),
            1.0,
            (-1920.0, 0.0, 1920.0, 1032.0),
        );
        let laptop = hover_card_rect(
            PhysicalPosition::new(2750.0, 1750.0),
            2.0,
            (0.0, 0.0, 2880.0, 1704.0),
        );
        assert_eq!((external.2, external.3), (380.0, 352.0));
        assert_eq!((laptop.2, laptop.3), (760.0, 704.0));
        assert!(laptop.0 >= 0.0 && laptop.1 + laptop.3 <= 1704.0);
    }

    #[test]
    fn tray_rect_converts_each_unit_without_double_scaling() {
        for logical_pos in [false, true] {
            for logical_size in [false, true] {
                let rect = tauri::Rect {
                    position: if logical_pos {
                        tauri::LogicalPosition::new(100.0, 200.0).into()
                    } else {
                        PhysicalPosition::new(200, 400).into()
                    },
                    size: if logical_size {
                        tauri::LogicalSize::new(20.0, 20.0).into()
                    } else {
                        tauri::PhysicalSize::new(40, 40).into()
                    },
                };
                assert_eq!(physical_tray_rect(rect, 2.0), (200.0, 400.0, 40.0, 40.0));
            }
        }
    }

    #[test]
    fn undersized_work_area_does_not_panic() {
        let rect = hover_card_rect(
            PhysicalPosition::new(80.0, 90.0),
            2.0,
            (0.0, 0.0, 100.0, 100.0),
        );
        assert!(rect.0.is_finite() && rect.1.is_finite());
    }
}

/// 卡片与托盘锚点的间隙（像素）：卡片底部距锚点上方留 24px，比系统默认更透气。
const HOVER_CARD_GAP: f64 = 24.0;

/// Work area, anchor, result and hit-test rectangles are all physical pixels.
/// Only the CSS dimensions and visual margins are logical pixels.
fn hover_card_rect(
    anchor: PhysicalPosition<f64>,
    scale: f64,
    work: (f64, f64, f64, f64),
) -> (f64, f64, f64, f64) {
    let width = (HOVER_CARD_W * scale).round();
    let height = (HOVER_CARD_H * scale).round();
    let margin = 8.0 * scale;
    let left = work.0 + margin;
    let top = work.1 + margin;
    let right = (work.0 + work.2 - margin - width).max(left);
    let bottom = (work.1 + work.3 - margin - height).max(top);
    let x = (anchor.x - width / 2.0).clamp(left, right);
    let above = anchor.y - height - HOVER_CARD_GAP * scale;
    let y = if above < top {
        anchor.y + 18.0 * scale
    } else {
        above
    };
    (x.round(), y.clamp(top, bottom).round(), width, height)
}

fn physical_tray_rect(rect: tauri::Rect, scale: f64) -> (f64, f64, f64, f64) {
    let position = rect.position.to_physical::<f64>(scale);
    let size = rect.size.to_physical::<f64>(scale);
    (position.x, position.y, size.width, size.height)
}

fn tray_rect(app: &AppHandle, cursor: PhysicalPosition<f64>) -> Option<(f64, f64, f64, f64)> {
    let rect = app.tray_by_id("main")?.rect().ok()??;
    // Windows Shell 返回物理矩形；常规采样不必跨 UI 线程查询显示器。
    let physical = matches!(rect.position, tauri::Position::Physical(_))
        && matches!(rect.size, tauri::Size::Physical(_));
    let scale = if physical {
        1.0
    } else {
        app.monitor_from_point(cursor.x, cursor.y)
            .ok()
            .flatten()?
            .scale_factor()
    };
    let rect = physical_tray_rect(rect, scale);
    (rect.2 > 0.0 && rect.3 > 0.0).then_some(rect)
}

/// hover_show 分档重发延迟（毫秒）：首帧在 do_show 立即发送，
/// 之后按此序列补发，覆盖 WebView2 冷启动加载期（可达 1s+）。
const RETRY_DELAYS: &[u64] = &[200, 500, 1000, 1800];

/// 托盘「暂停监控」菜单项句柄：update_tray 每秒按全局暂停状态维护文案
/// （"暂停监控" ↔ "恢复监控"）。create_tray 时写入，之后只读使用。
static PAUSE_ITEM: Mutex<Option<MenuItem<tauri::Wry>>> = Mutex::new(None);

/// 当前发现的新版本号（无发现则 None）：tooltip 追加「（有新版本 v{v}）」提示；
/// 托盘右键菜单不再放更新入口，更新操作统一走主界面更新页。
static UPDATE_VERSION: Mutex<Option<String>> = Mutex::new(None);

/// 悬停卡片诊断日志：仅异常 / 兜底 / 启动落盘（正常显示隐藏不写，避免长期持续写盘）。
/// 路径：%TEMP%\niuma_timer_hover.log
fn hover_log(msg: &str) {
    use std::io::Write;
    let p = std::env::temp_dir().join("niuma_timer_hover.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
    {
        let _ = writeln!(f, "{msg}");
    }
}

/// 工作线程收到的消息（全部来自主线程托盘事件 / 前端握手，自身不持有状态）。
#[derive(Debug)]
enum HoverMsg {
    /// 进入托盘，立即复核实际光标位置
    Enter,
    /// 托盘内移动
    Move,
    /// 离开托盘，立即复核并隐藏
    Leave,
    /// 单击托盘（立即隐藏，与系统 tooltip 点击即消失一致）
    Click,
    /// 左键双击（隐藏 + 打开主界面）
    DoubleClick,
    /// 前端 hover_card.html 加载完成握手
    Ready,
    /// 菜单项点击（隐藏，重新移入后才允许显示）
    MenuHide,
}

/// 唯一的显隐控制器：前端不再自行 hide 窗口，避免旧淡出回调关掉新一轮提示。
#[derive(Default)]
struct HoverController {
    state: hover_state::HoverState,
    card_rect: Option<(f64, f64, f64, f64)>,
    retry_base: Option<Instant>,
    retry_idx: usize,
}

impl HoverController {
    fn card_pos(app: &AppHandle, anchor: PhysicalPosition<f64>) -> Option<(f64, f64, f64, f64)> {
        let monitor = app.monitor_from_point(anchor.x, anchor.y).ok().flatten()?;
        let area = monitor.work_area();
        let work = (
            area.position.x as f64,
            area.position.y as f64,
            area.size.width as f64,
            area.size.height as f64,
        );
        Some(hover_card_rect(anchor, monitor.scale_factor(), work))
    }

    /// 分开采样触发区（托盘）与保持区（已显示卡片），隐藏卡片不能反向触发弹出。
    fn pointer_sample(
        &self,
        app: &AppHandle,
    ) -> (Option<bool>, bool, Option<PhysicalPosition<f64>>) {
        let Some((x, y)) = crate::win::cursor_pos() else {
            return (None, false, None);
        };
        let pos = PhysicalPosition::new(x as f64, y as f64);
        let on_card = self.state.is_shown()
            && self.card_rect.is_some_and(|rect| {
                hover_state::in_card((pos.x, pos.y), rect, rect.2 / HOVER_CARD_W)
            })
            && app
                .get_webview_window("hover_card")
                .is_some_and(|w| w.is_visible().unwrap_or(false));
        let Some(rect) = tray_rect(app, pos) else {
            return (None, on_card, None);
        };
        let inside =
            hover_state::in_tray((pos.x, pos.y), rect) && crate::win::cursor_over_taskbar(x, y);
        (
            Some(inside),
            on_card,
            Some(PhysicalPosition::new(
                rect.0 + rect.2 / 2.0,
                rect.1 + rect.3 / 2.0,
            )),
        )
    }

    fn do_show(&mut self, app: &AppHandle) -> bool {
        let Some(w) = ensure_hover_card(app) else {
            return false;
        };
        // 首次创建 WebView 可能耗时；创建后重新检查，不能使用数百毫秒前的 Enter。
        let (inside, _, anchor) = self.pointer_sample(app);
        if inside != Some(true) || crate::win::tooltip_input_blocked() != Some(false) {
            return false;
        }
        let Some(rect) = anchor.and_then(|a| Self::card_pos(app, a)) else {
            return false;
        };
        let (x, y, width, height) = rect;
        if self.card_rect != Some(rect) {
            if let Err(e) = w
                .set_position(PhysicalPosition::new(x, y))
                .and_then(|_| w.set_size(tauri::PhysicalSize::new(width as u32, height as u32)))
            {
                hover_log(&format!("[hover_card] 设置位置/尺寸失败: {e}"));
                return false;
            }
            self.card_rect = Some(rect);
        }
        if let Err(e) = w.show() {
            hover_log(&format!("[hover_card] 显示失败: {e}"));
            return false;
        }
        self.emit_show(app);
        self.retry_base = Some(Instant::now());
        self.retry_idx = 0;
        true
    }

    fn emit_show(&self, app: &AppHandle) {
        if let Some(w) = app.get_webview_window("hover_card") {
            let st = crate::get_status(app.state::<crate::AppState>().inner());
            let _ = w.emit("hover_data", st);
            let _ = w.emit("hover_show", ());
        }
    }

    fn do_hide(&mut self, app: &AppHandle) {
        self.card_rect = None;
        self.retry_base = None;
        self.retry_idx = 0;
        if let Some(w) = app.get_webview_window("hover_card") {
            let _ = w.emit("hover_hide", ());
            // 状态机决定收起后立即隐藏，不留下跨越下一次悬停的淡出任务。
            let _ = w.hide();
        }
    }

    fn poll(&mut self, app: &AppHandle) {
        let enabled =
            sync::lock(&app.state::<crate::AppState>().config, "state.config").tray_hover_card;
        if !enabled {
            if self.state.reset() == hover_state::Action::Hide {
                self.do_hide(app);
            }
            return;
        }
        let blocked = crate::win::tooltip_input_blocked().unwrap_or(true);
        let (inside, on_card, _) = if blocked {
            (None, false, None)
        } else {
            self.pointer_sample(app)
        };
        match self.state.observe(Instant::now(), inside, on_card, blocked) {
            hover_state::Action::Show => {
                if !self.do_show(app) {
                    self.state.reset();
                    self.do_hide(app);
                }
            }
            hover_state::Action::Hide => self.do_hide(app),
            hover_state::Action::None => {}
        }
        // 事件持续到达也会运行定时任务，不再要求通道安静后才显示。
        if self.state.is_shown() {
            if let Some(base) = self.retry_base {
                if self.retry_idx >= RETRY_DELAYS.len() {
                    self.retry_base = None;
                } else if Instant::now()
                    >= base + Duration::from_millis(RETRY_DELAYS[self.retry_idx])
                {
                    self.emit_show(app);
                    self.retry_idx += 1;
                }
            }
        }
    }

    fn handle(&mut self, msg: HoverMsg, app: &AppHandle) {
        match msg {
            // 坐标从系统重新读取，事件只作为立即采样的触发器，避免处理队列里的旧坐标。
            HoverMsg::Enter | HoverMsg::Move | HoverMsg::Leave => {}
            HoverMsg::Click | HoverMsg::MenuHide => {
                self.state.suppress();
                self.do_hide(app);
            }
            HoverMsg::DoubleClick => {
                self.state.suppress();
                self.do_hide(app);
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.unminimize();
                    let _ = w.show();
                    let _ = w.set_focus();
                    let w2 = w.clone();
                    let _ = std::thread::Builder::new()
                        .name("niuma-focus-retry".to_string())
                        .spawn(move || {
                            std::thread::sleep(Duration::from_millis(300));
                            let _ = w2.set_focus();
                        });
                }
            }
            HoverMsg::Ready => {
                // 页面就绪不能越过延迟、离开或右键抑制；先检查真实位置和菜单状态。
                self.poll(app);
                if self.state.is_shown() {
                    self.emit_show(app);
                }
            }
        }
    }

    fn next_wakeup(&self) -> Duration {
        let now = Instant::now();
        let mut delay = self.state.next_wakeup(now);
        if let Some(base) = self.retry_base {
            if let Some(ms) = RETRY_DELAYS.get(self.retry_idx) {
                delay =
                    delay.min((base + Duration::from_millis(*ms)).saturating_duration_since(now));
            }
        }
        delay
    }
}

/// 单个 worker 串行处理消息和真实光标采样。100ms 低频兜底覆盖 Shell 漏发事件。
fn spawn_hover_worker(app: AppHandle, rx: mpsc::Receiver<HoverMsg>) {
    let _ = std::thread::Builder::new()
        .name("niuma-hover".to_string())
        .spawn(move || {
            let mut ctrl = HoverController::default();
            hover_log("[hover_card] 工作线程已启动");
            loop {
                match rx.recv_timeout(ctrl.next_wakeup()) {
                    Ok(msg) => ctrl.handle(msg, &app),
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
                ctrl.poll(&app);
            }
        });
}

/// 创建托盘图标、菜单；悬停卡片窗口首次悬停时懒创建（启动路径不创建额外窗口）。
/// 记录「发现新版本」，供 tooltip 追加提示；同版本重复调用无副作用。
/// 托盘创建早晚都无妨：这里只写状态，不触碰菜单。
pub fn set_update_available(version: &str) {
    let mut cur = sync::lock(&UPDATE_VERSION, "tray::UPDATE_VERSION");
    if cur.as_deref() == Some(version) {
        return;
    }
    *cur = Some(version.to_string());
}

pub fn create_tray(app: &App) -> tauri::Result<TrayIcon> {
    let settings = MenuItem::with_id(app, "settings", "主界面", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "刷新工作日", true, None::<&str>)?;
    let pause_item = MenuItem::with_id(app, "pause", "暂停监控", true, None::<&str>)?;
    // 阶段 A：手动检查更新入口（结果落到 viewUpdate；不受 update_auto_check 限制）
    let check_update = MenuItem::with_id(app, "check-update", "检查更新", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    *sync::lock(&PAUSE_ITEM, "tray::PAUSE_ITEM") = Some(pause_item.clone());
    let menu = Menu::with_items(
        app,
        &[&settings, &refresh, &check_update, &pause_item, &quit],
    )?;

    let (tx, rx) = mpsc::channel::<HoverMsg>();

    // 前端页面就绪握手：hover_card.html 加载完成后 emit "hover_ready"，
    // 转发给工作线程补一次定位与淡入
    let tx_ready = tx.clone();
    let _ = app.listen("hover_ready", move |_event| {
        let _ = tx_ready.send(HoverMsg::Ready);
    });
    // 前端脚本错误上报，便于排查"不显示"类问题
    let _ = app.listen("hover_error", |e| {
        hover_log(&format!("[hover_card] 前端错误: {}", e.payload()));
    });

    // 启动工作线程（Actor）：独自持有全部状态与定时逻辑
    spawn_hover_worker(app.handle().clone(), rx);

    let tx_menu = tx.clone();
    let tx_tray = tx.clone();

    let icon = static_icon();
    let tray = TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("牛马计时器启动中…")
        .menu(&menu)
        // Windows 默认左键点击也弹菜单，导致左键双击直接弹右键菜单而非打开主界面。
        // 显式关闭后左键只产生 Click/DoubleClick，右键仍正常弹菜单
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| {
            // 点击任意菜单项立即收掉悬停卡片，离开并重新移入才恢复
            let _ = tx_menu.send(HoverMsg::MenuHide);
            if event.id == MenuId::new("settings") {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.unminimize();
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            } else if event.id == MenuId::new("refresh") {
                crate::spawn_holiday_refresh(app.clone());
            } else if event.id == MenuId::new("pause") {
                crate::toggle_pause(app);
            } else if event.id == MenuId::new("check-update") {
                // 只是「把主窗拉出来 + 通知前端切到更新视图」，
                // 真正的检查/更新逻辑在前端点按钮后走 A7 的命令。
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.unminimize();
                    let _ = w.show();
                    let _ = w.set_focus();
                }
                let _ = app.emit("update-view-requested", ());
            } else if event.id == MenuId::new("quit") {
                app.exit(0);
            }
        })
        .on_tray_icon_event(move |_tray, event| match event {
            // 进入托盘：延迟一段时间（鼠标仍停留）才显示，模仿系统原生 tooltip
            TrayIconEvent::Enter { .. } => {
                let _ = tx_tray.send(HoverMsg::Enter);
            }
            // 托盘内移动：立即采样，隐藏时也能补上漏发的 Enter
            TrayIconEvent::Move { .. } => {
                let _ = tx_tray.send(HoverMsg::Move);
            }
            // 离开托盘：立即采样，取消待显或隐藏卡片
            TrayIconEvent::Leave { .. } => {
                let _ = tx_tray.send(HoverMsg::Leave);
            }
            // 左键双击：立即隐藏 + 打开主界面
            TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => {
                let _ = tx_tray.send(HoverMsg::DoubleClick);
            }
            // 单击（右键菜单 / 左键单击）：立即隐藏
            TrayIconEvent::Click { .. } => {
                let _ = tx_tray.send(HoverMsg::Click);
            }
            _ => {}
        })
        .build(app)?;
    Ok(tray)
}

/// 懒创建 hover_card 窗口（首次悬停时才创建，发生在工作线程，不影响主窗口渲染；
/// 创建失败仅禁用悬停卡片，不影响主程序）。调用方唯一（worker），无需并发保护。
fn ensure_hover_card(app: &AppHandle) -> Option<WebviewWindow> {
    if let Some(w) = app.get_webview_window("hover_card") {
        return Some(w);
    }
    let created = WebviewWindowBuilder::new(
        app,
        "hover_card",
        WebviewUrl::App("hover_card.html?v=f94d1679".into()),
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
    .focused(false)
    .focusable(false)
    .build();
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

/// 更新托盘的 tooltip / 悬停卡片
/// - 彩色卡片开启：清空系统 tooltip（避免双显），卡片可见时每秒推送实时数据
/// - 彩色卡片关闭：恢复系统原生 tooltip
///
/// 直接查询窗口可见性，不依赖任何全局状态。
pub fn update_tray(app: &AppHandle, status: &DayStatus) {
    let cfg = sync::lock(&app.state::<crate::AppState>().config, "state.config").clone();
    if let Some(tray) = app.tray_by_id("main") {
        if cfg.tray_hover_card {
            let _ = tray.set_tooltip::<&str>(None);
        } else {
            // 阶段 A：有可用更新时在 tooltip 追加提示（无更新则后缀为空，行为不变）
            let suffix = sync::lock(&UPDATE_VERSION, "tray::UPDATE_VERSION")
                .as_deref()
                .map(|v| format!("（有新版本 v{v}）"))
                .unwrap_or_default();
            let _ = tray.set_tooltip(Some(&format!("{}{suffix}", status.tooltip)));
        }
    }
    // 暂停菜单项文案跟随全局状态；先比对现文案，避免每秒对原生菜单做无谓的 set_text
    if let Some(item) = sync::lock(&PAUSE_ITEM, "tray::PAUSE_ITEM").as_ref() {
        let want = if status.paused {
            "恢复监控"
        } else {
            "暂停监控"
        };
        if item.text().map(|t| t != want).unwrap_or(true) {
            let _ = item.set_text(want);
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
