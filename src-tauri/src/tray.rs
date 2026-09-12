//! 托盘悬停卡片状态机（P3 重写）
//!
//! 旧实现用 9 个全局 static + 看门狗线程 + 延迟计时器 + 点击冷却 + 分档重发，
//! 多个线程并发读写同一批状态，难以推理。本版本改为 **actor 模型**：
//! - 托盘事件 / hover_ready 握手只向通道发送消息，不做任何状态变更；
//! - 单个工作线程独占 `HoverController`（持有全部状态），用 `recv_timeout`
//!   统一驱动"延迟显示 / 看门狗轮询 / 淡出兜底 / 分档重发"四类定时任务；
//! - 空闲时阻塞 `recv`，无空转；状态变更只发生在一个线程，结构上不可能数据竞争。
//! 前端握手协议（hover_ready / hover_show / hover_hide / hover_data）保持不变。

use crate::sync;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use tauri::Emitter;
use tauri::Listener;
use tauri::Manager;
use tauri::menu::{Menu, MenuId, MenuItem};
use tauri::tray::{MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::calc::DayStatus;
use crate::icon_render::static_icon;

/// 彩色悬停卡片尺寸（逻辑像素）。高度 316 = 卡片 300（时间轴/战果行/补账与预告行）+ 边距
const HOVER_CARD_W: f64 = 320.0;
const HOVER_CARD_H: f64 = 316.0;

/// 延迟显示时长（毫秒）：进入托盘后等待该时长，若仍停留才显示，
/// 模仿系统原生 tooltip 的延迟出现，避免划过托盘就弹窗。
const HOVER_SHOW_DELAY_MS: u64 = 400;

/// 点击冷却期（毫秒）：点击托盘后该时长内不自动弹出卡片，
/// 避免"右键弹系统菜单 → 鼠标仍停托盘触发 Enter → 卡片重新弹出遮挡菜单"。
const CLICK_COOLDOWN_MS: u64 = 1500;

/// 看门狗轮询间隔（毫秒）：卡片可见时每 150ms 检查一次鼠标是否离开活动区域。
const WATCHDOG_INTERVAL_MS: u64 = 150;

/// 最长驻留时长（毫秒）：卡片持续显示超过该时长即强制隐藏。
/// 作为看门狗 / Leave 等隐藏路径全部失效时的终极安全网，避免偶发故障导致卡片永久滞留。
const MAX_VISIBLE_MS: u64 = 20000;

/// 卡片与托盘锚点的间隙（像素）：卡片底部距锚点上方留 24px，比系统默认更透气。
const HOVER_CARD_GAP: f64 = 24.0;

/// 活动区域余量（像素）：判断鼠标是否仍在托盘图标 / 卡片内，防边缘抖动误隐藏。
const REGION_PAD: f64 = 12.0;

/// hover_show 分档重发延迟（毫秒）：首帧在 do_show 立即发送，
/// 之后按此序列补发，覆盖 WebView2 冷启动加载期（可达 1s+）。
const RETRY_DELAYS: &[u64] = &[200, 500, 1000, 1800];

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 悬停卡片诊断日志：仅异常 / 兜底 / 启动落盘（正常显示隐藏不写，避免长期持续写盘）。
/// 路径：%TEMP%\niuma_timer_hover.log
fn hover_log(msg: &str) {
    use std::io::Write;
    let p = std::env::temp_dir().join("niuma_timer_hover.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{msg}");
    }
}

/// 工作线程收到的消息（全部来自主线程托盘事件 / 前端握手，自身不持有状态）。
#[derive(Debug)]
enum HoverMsg {
    /// 进入托盘（pos 为光标位置，仅作锚点回退用）
    Enter(PhysicalPosition<f64>),
    /// 托盘内移动
    Move(PhysicalPosition<f64>),
    /// 离开托盘（不立即隐藏，交给看门狗去抖）
    Leave,
    /// 单击托盘（立即隐藏，与系统 tooltip 点击即消失一致）
    Click,
    /// 左键双击（淡出 + 打开主界面）
    DoubleClick,
    /// 前端 hover_card.html 加载完成握手
    Ready,
    /// 菜单项点击（仅隐藏，不进入点击冷却，保持与原行为一致）
    MenuHide,
}

/// 卡片阶段：唯一真相来源，所有状态机转移都围绕它。
#[derive(Default)]
enum Phase {
    /// 空闲：卡片未显示、无待显示任务。
    #[default]
    Hidden,
    /// 已进入托盘，等待延迟到期后校验鼠标仍停留才显示。
    Pending {
        show_at: Instant,
    },
    /// 卡片可见。fading=true 表示正在淡出（看门狗暂停、不重发），
    /// fade_deadline 为强制隐藏窗口的兜底时刻。
    Shown {
        fading: bool,
        fade_deadline: Option<Instant>,
    },
}

/// 悬停卡片控制器：工作线程独占，持有全部可变状态。
#[derive(Default)]
struct HoverController {
    /// 托盘图标中心锚点缓存（rect 偶发失败时退回，避免卡片跟手抖动）
    tray_center: Option<PhysicalPosition<f64>>,
    /// 最近一次悬停锚点（hover_ready 补显 + 延迟显示定位用）
    pending_pos: Option<PhysicalPosition<f64>>,
    /// 上次点击时间戳（毫秒），用于点击冷却判定
    last_click_ms: u64,
    /// 卡片当前屏幕矩形（x,y,w,h），用于"鼠标是否仍在卡片上"判定
    card_rect: Option<(f64, f64, f64, f64)>,
    /// 当前阶段
    phase: Phase,
    /// 看门狗连续"离开"轮询计数（去抖：连续 2 次确认才隐藏）
    gone_polls: u32,
    /// 卡片进入 Shown 阶段的时刻，用于"最长驻留硬上限"计时
    shown_at: Option<Instant>,
    /// hover_show 分档重发基准时刻与下一档索引
    retry_base: Option<Instant>,
    retry_idx: usize,
}

impl HoverController {
    /// 是否处于点击冷却期
    fn in_click_cooldown(&self) -> bool {
        now_ms().saturating_sub(self.last_click_ms) < CLICK_COOLDOWN_MS
    }

    /// 计算悬停卡片左上角坐标：水平居中于锚点正上方，按主屏钳制防溢出。
    fn card_pos(app: &AppHandle, anchor: PhysicalPosition<f64>) -> (f64, f64) {
        let mut x = anchor.x - HOVER_CARD_W / 2.0;
        let mut y = anchor.y - HOVER_CARD_H - HOVER_CARD_GAP;
        if let Ok(Some(m)) = app.primary_monitor() {
            let size = m.size();
            let sw = size.width as f64;
            let sh = size.height as f64;
            if y < 8.0 {
                y = anchor.y + 18.0;
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
        (x, y)
    }

    /// 计算托盘图标中心锚点：卡片水平居中于此（精确居中，不随进入方向偏移）。
    /// rect 成功时刷新缓存；失败退回缓存中心；再不行退光标位置。
    fn tray_anchor(&mut self, app: &AppHandle, fallback: PhysicalPosition<f64>) -> PhysicalPosition<f64> {
        if let Some(tray) = app.tray_by_id("main") {
            if let Ok(Some(r)) = tray.rect() {
                let (cx, cy) = match (r.position, r.size) {
                    (tauri::Position::Physical(p), tauri::Size::Physical(s)) => (
                        p.x as f64 + s.width as f64 / 2.0,
                        p.y as f64 + s.height as f64 / 2.0,
                    ),
                    (tauri::Position::Logical(p), tauri::Size::Logical(s)) => (
                        p.x + s.width / 2.0,
                        p.y + s.height / 2.0,
                    ),
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
                self.tray_center = Some(c);
                return c;
            }
        }
        if let Some(c) = self.tray_center {
            return c;
        }
        fallback
    }

    /// 鼠标是否仍在托盘图标范围内（带余量）。rect 拿不到时退回锚点 ±40px；
    /// 完全未知时返回 true（不误隐藏）。
    fn mouse_near_tray(&self, app: &AppHandle, pos: PhysicalPosition<f64>) -> bool {
        const PAD: f64 = REGION_PAD;
        if let Some(tray) = app.tray_by_id("main") {
            if let Ok(Some(r)) = tray.rect() {
                let (rx, ry, rw, rh) = match (r.position, r.size) {
                    (tauri::Position::Physical(p), tauri::Size::Physical(s)) => (
                        p.x as f64,
                        p.y as f64,
                        s.width as f64,
                        s.height as f64,
                    ),
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
        if let Some(a) = self.pending_pos {
            return (pos.x - a.x).abs() <= 40.0 && (pos.y - a.y).abs() <= 40.0;
        }
        true
    }

    /// 鼠标是否在"保持卡片可见"的活动区域内：托盘图标内 或 悬停卡片矩形内。
    fn mouse_in_active_region(&self, app: &AppHandle, pos: PhysicalPosition<f64>) -> bool {
        if self.mouse_near_tray(app, pos) {
            return true;
        }
        if let Some((cx, cy, cw, ch)) = self.card_rect {
            const PAD: f64 = REGION_PAD;
            return pos.x >= cx - PAD
                && pos.x <= cx + cw + PAD
                && pos.y >= cy - PAD
                && pos.y <= cy + ch + PAD;
        }
        false
    }

    /// 显示卡片（淡入 + 首帧 hover_show + 分档重发调度）。幂等：已显示则刷新。
    fn do_show(&mut self, app: &AppHandle, anchor: PhysicalPosition<f64>) {
        let cfg = sync::lock(&app.state::<crate::AppState>().config, "state.config").clone();
        if !cfg.tray_hover_card {
            return;
        }
        self.pending_pos = Some(anchor);
        let Some(w) = ensure_hover_card(app) else {
            return;
        };
        let (x, y) = Self::card_pos(app, anchor);
        let rect = (x, y, HOVER_CARD_W, HOVER_CARD_H);
        let prev = self.card_rect;
        self.card_rect = Some(rect);
        // 位置未变则不重定位：避免对透明分层窗口反复 set_position 引发抖动/重绘闪烁
        if prev != Some(rect) {
            let _ = w.set_position(PhysicalPosition::new(x, y));
        }
        let was_visible = w.is_visible().unwrap_or(false);
        if !was_visible {
            // show() 不抢焦点（Windows SW_SHOW），避免干扰用户操作
            let _ = w.show();
        }
        // 立即推送一帧数据（此后由 update_tray 每秒续推）
        let st = crate::get_status(app.state::<crate::AppState>().inner());
        let _ = w.emit("hover_data", st);
        // 淡入事件：立即 + 分档重试覆盖 WebView2 首次冷启动加载期；
        // 页面加载超过重试窗口时，由 hover_ready 握手兜底补显
        let _ = w.emit("hover_show", ());
        self.retry_base = Some(Instant::now());
        self.retry_idx = 0;
        self.phase = Phase::Shown {
            fading: false,
            fade_deadline: None,
        };
        self.shown_at = Some(Instant::now());
        self.gone_polls = 0;
    }

    /// 鼠标离开托盘：播放淡出动画，动画结束后由页面自行隐藏窗口。
    fn do_hide(&mut self, app: &AppHandle) {
        self.card_rect = None;
        if let Some(w) = app.get_webview_window("hover_card") {
            if w.is_visible().unwrap_or(false) {
                let _ = w.emit("hover_hide", ());
                // 兜底：页面动画异常时强制隐藏； fade_deadline 到点由 on_timeout 处理
                self.phase = Phase::Shown {
                    fading: true,
                    fade_deadline: Some(Instant::now() + Duration::from_millis(450)),
                };
                return;
            }
        }
        self.phase = Phase::Hidden;
        self.gone_polls = 0;
    }

    /// 立即隐藏卡片（不走淡出）：点击托盘时调用，与系统原生 tooltip 点击即消失一致。
    fn do_hide_instant(&mut self, app: &AppHandle) {
        self.card_rect = None;
        self.retry_base = None;
        self.retry_idx = 0;
        if let Some(w) = app.get_webview_window("hover_card") {
            let _ = w.emit("hover_hide", ());
            let _ = w.hide();
        }
        self.phase = Phase::Hidden;
        self.gone_polls = 0;
    }

    /// 处理一条消息（全部在 worker 线程，独占 self）
    fn handle(&mut self, msg: HoverMsg, app: &AppHandle) {
        match msg {
            HoverMsg::Enter(pos) => {
                // 锚点每次都刷新（图标可能在任务栏中移动）；阶段按现状收敛：
                //  - Pending：只刷新锚点，保持原到期时刻——Windows 下 tao 在图标内
                //    移动会偶发重复上报 Enter，若重置计时会让 400ms 延迟永远到不了点
                //  - Shown：只刷新锚点与数据，保持展示——重复 Enter 不得打断看门狗
                //    节拍，更不得把状态降级回 Pending 造成 400ms 盲区
                //  - Hidden：正常进入延迟显示；若处于点击冷却期，则安排在冷却
                //    结束时刻显示——右键菜单/点击后鼠标保持悬停时，冷却一过卡片
                //    立刻回来，无需「移出再移入」；中途移走由 Leave 取消
                let anchor = self.tray_anchor(app, pos);
                match self.phase {
                    Phase::Pending { show_at } => {
                        self.pending_pos = Some(anchor);
                        self.phase = Phase::Pending { show_at };
                    }
                    Phase::Shown { .. } => {
                        self.pending_pos = Some(anchor);
                        if let Some(w) = app.get_webview_window("hover_card") {
                            if w.is_visible().unwrap_or(false) {
                                let st =
                                    crate::get_status(app.state::<crate::AppState>().inner());
                                let _ = w.emit("hover_data", st);
                            }
                        }
                    }
                    Phase::Hidden => {
                        self.pending_pos = Some(anchor);
                        if self.in_click_cooldown() {
                            let remain = CLICK_COOLDOWN_MS
                                .saturating_sub(now_ms().saturating_sub(self.last_click_ms));
                            self.phase = Phase::Pending {
                                show_at: Instant::now() + Duration::from_millis(remain),
                            };
                        } else {
                            self.phase = Phase::Pending {
                                show_at: Instant::now()
                                    + Duration::from_millis(HOVER_SHOW_DELAY_MS),
                            };
                        }
                    }
                }
            }
            HoverMsg::Move(pos) => {
                if matches!(self.phase, Phase::Pending { .. }) {
                    // 未显示期间只更新待显位置，真正的显示由延迟计时器统一处理
                    self.pending_pos = Some(self.tray_anchor(app, pos));
                } else if matches!(self.phase, Phase::Shown { .. }) {
                    // 已可见时只刷新实时数据，绝不重复重定位（防抖动）
                    if let Some(w) = app.get_webview_window("hover_card") {
                        if w.is_visible().unwrap_or(false) {
                            let st =
                                crate::get_status(app.state::<crate::AppState>().inner());
                            let _ = w.emit("hover_data", st);
                        }
                    }
                }
            }
            HoverMsg::Leave => {
                // 待显示直接取消（用户还没等到卡片，无需任何过渡）
                if matches!(self.phase, Phase::Pending { .. }) {
                    self.phase = Phase::Hidden;
                    self.pending_pos = None;
                    return;
                }
                // 已显示：Leave 是确定性的离开事件，立即淡出——不再等看门狗
                // 两拍（最多 ~300ms+）。「Windows 托盘边缘偶发抖动的假 Leave」
                // 用光标真实位置当场复核：确实已离开才隐藏，仍在区域内则交回
                // 看门狗继续值守；光标位置拿不到时维持原状兜底
                if matches!(self.phase, Phase::Shown { fading: false, .. }) {
                    match app.cursor_position() {
                        Ok(pos) if !self.mouse_in_active_region(app, pos) => self.do_hide(app),
                        _ => { /* 假 Leave 或光标不可知：看门狗兜底 */ }
                    }
                }
            }
            HoverMsg::Click => {
                // 记录点击时间（进入冷却期），立即隐藏（与系统 tooltip 一致）
                self.last_click_ms = now_ms();
                self.phase = Phase::Hidden;
                self.pending_pos = None;
                self.do_hide_instant(app);
            }
            HoverMsg::DoubleClick => {
                self.last_click_ms = now_ms();
                self.do_hide(app);
                // 打开主界面（最小化状态需先 unminimize）
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.unminimize();
                    let _ = w.show();
                    let _ = w.set_focus();
                    // 延迟再抢一次焦点：绕过 Windows 前台锁定
                    let w2 = w.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(300));
                        let _ = w2.set_focus();
                    });
                }
            }
            HoverMsg::Ready => {
                // 页面就绪：若鼠标仍悬停在托盘上（pending_pos 存在），补一次定位 + 淡入。
                // 这是"首次悬停事件丢失"的最终兜底，不依赖任何窗口可见性猜测。
                if let Some(pos) = self.pending_pos {
                    self.do_show(app, pos);
                }
            }
            HoverMsg::MenuHide => {
                // 菜单项点击：仅隐藏，不进入点击冷却（保持与原行为一致）
                self.phase = Phase::Hidden;
                self.pending_pos = None;
                self.do_hide_instant(app);
            }
        }
    }

    /// 定时任务到点（由 recv_timeout 触发）：驱动延迟显示 / 看门狗 / 淡出兜底 / 重发。
    fn on_timeout(&mut self, app: &AppHandle) {
        let now = Instant::now();
        // 复制阶段标记，避免长期借用 self.phase 阻碍调用其他方法
        let is_pending = matches!(self.phase, Phase::Pending { .. });
        let is_shown = matches!(self.phase, Phase::Shown { .. });
        let is_fading = matches!(self.phase, Phase::Shown { fading: true, .. });
        // 复制定时字段
        let show_at = if let Phase::Pending { show_at } = &self.phase {
            Some(*show_at)
        } else {
            None
        };
        let fade_deadline = if let Phase::Shown { fade_deadline, .. } = &self.phase {
            *fade_deadline
        } else {
            None
        };

        if is_pending {
            if let Some(sa) = show_at {
                if now >= sa {
                    // 到点：Enter 已证明鼠标在托盘，直接用锚点显示，不再重查
                    // cursor_position()——该调用在 Windows 下偶发失败，曾导致偶发不显示。
                    // 中途离开由 Leave 取消，漏网的由看门狗（每 150ms 轮询）兜底隐藏。
                    if !self.in_click_cooldown() {
                        if let Some(anchor) = self.pending_pos {
                            self.do_show(app, anchor);
                            return;
                        }
                    }
                    // 无待显锚点（理论不该发生）：取消待显
                    self.phase = Phase::Hidden;
                    self.pending_pos = None;
                }
            }
            return;
        }

        if is_shown {
            if is_fading {
                // 淡出兜底：到点强制隐藏窗口
                if let Some(d) = fade_deadline {
                    if now >= d {
                        if let Some(w) = app.get_webview_window("hover_card") {
                            let _ = w.hide();
                        }
                        self.phase = Phase::Hidden;
                        self.card_rect = None;
                        self.retry_base = None;
                    }
                }
                // 淡出中不轮询看门狗、不重发 hover_show
                return;
            }
            // 最长驻留硬上限：任何隐藏路径（看门狗 / Leave）失效的终极安全网，
            // 卡片持续显示超过该时长强制隐藏，避免偶发故障导致卡片永久滞留。
            if let Some(st) = self.shown_at {
                if now.saturating_duration_since(st) >= Duration::from_millis(MAX_VISIBLE_MS) {
                    hover_log(&format!(
                        "[hover_card] 超过最长驻留 {}ms，强制隐藏",
                        MAX_VISIBLE_MS
                    ));
                    self.do_hide(app);
                    return;
                }
            }
            // 看门狗：轮询鼠标是否仍在活动区域，连续 2 次离开才隐藏（去抖）
            match app.cursor_position() {
                Ok(pos) => {
                    if self.mouse_in_active_region(app, pos) {
                        self.gone_polls = 0;
                    } else {
                        self.gone_polls += 1;
                        if self.gone_polls >= 2 {
                            self.do_hide(app);
                            self.gone_polls = 0;
                            return;
                        }
                    }
                }
                Err(_) => {
                    // 取不到光标位置：不累加也不清零，留待下次轮询继续裁决，
                    // 避免偶发失败清零去抖计数导致"连续 2 次离开"永远无法满足，卡片永久不隐藏
                }
            }
            // 分档重发 hover_show（覆盖页面冷启动加载期）
            if let Some(base) = self.retry_base {
                if self.retry_idx < RETRY_DELAYS.len() {
                    let t = base + Duration::from_millis(RETRY_DELAYS[self.retry_idx]);
                    if now >= t {
                        if let Some(w) = app.get_webview_window("hover_card") {
                            let _ = w.emit("hover_show", ());
                        }
                        self.retry_idx += 1;
                    }
                } else {
                    self.retry_base = None;
                }
            }
        }
    }

    /// 计算下次唤醒的超时：None 表示空闲（阻塞等待事件）；Some 为最近定时任务时刻。
    fn next_wakeup(&self) -> Option<Duration> {
        match &self.phase {
            Phase::Hidden => None,
            Phase::Pending { show_at } => {
                Some(show_at.saturating_duration_since(Instant::now()))
            }
            Phase::Shown { fading, fade_deadline } => {
                let mut min = Duration::from_millis(WATCHDOG_INTERVAL_MS);
                // 最长驻留上限也纳入唤醒时刻，保证到点一定触发强制隐藏
                if let Some(st) = self.shown_at {
                    let remain = (st + Duration::from_millis(MAX_VISIBLE_MS))
                        .saturating_duration_since(Instant::now());
                    min = min.min(remain);
                }
                if *fading {
                    if let Some(d) = fade_deadline {
                        min = min.min(d.saturating_duration_since(Instant::now()));
                    }
                }
                if let Some(base) = self.retry_base {
                    if self.retry_idx < RETRY_DELAYS.len() {
                        let t = base + Duration::from_millis(RETRY_DELAYS[self.retry_idx]);
                        min = min.min(t.saturating_duration_since(Instant::now()));
                    }
                }
                Some(min)
            }
        }
    }
}

/// 启动悬停卡片工作线程（actor）：独占 HoverController，统一裁决所有定时任务。
/// 通道断开（所有发送端释放）时退出。
fn spawn_hover_worker(app: AppHandle, rx: mpsc::Receiver<HoverMsg>) {
    std::thread::spawn(move || {
        let mut ctrl = HoverController::default();
        hover_log("[hover_card] 工作线程已启动");
        loop {
            match ctrl.next_wakeup() {
                None => {
                    // 空闲：阻塞等待事件，无空转
                    match rx.recv() {
                        Ok(msg) => ctrl.handle(msg, &app),
                        Err(_) => break, // 通道关闭
                    }
                }
                Some(timeout) => match rx.recv_timeout(timeout) {
                    Ok(msg) => ctrl.handle(msg, &app),
                    Err(mpsc::RecvTimeoutError::Timeout) => ctrl.on_timeout(&app),
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                },
            }
        }
    });
}

/// 创建托盘图标、菜单；悬停卡片窗口首次悬停时懒创建（启动路径不创建额外窗口）。
pub fn create_tray(app: &App) -> tauri::Result<TrayIcon> {
    let settings = MenuItem::with_id(app, "settings", "主界面", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "刷新工作日", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&settings, &refresh, &quit])?;

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
            // 点击任意菜单项立即收掉悬停卡片（双保险），不进入点击冷却
            let _ = tx_menu.send(HoverMsg::MenuHide);
            if event.id == MenuId::new("settings") {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.unminimize();
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            } else if event.id == MenuId::new("refresh") {
                crate::spawn_holiday_refresh(app.clone());
            } else if event.id == MenuId::new("quit") {
                app.exit(0);
            }
        })
        .on_tray_icon_event(move |_tray, event| match event {
            // 进入托盘：延迟一段时间（鼠标仍停留）才显示，模仿系统原生 tooltip
            TrayIconEvent::Enter { position, .. } => {
                let _ = tx_tray.send(HoverMsg::Enter(position));
            }
            // 托盘内移动：已可见只刷数据，未显示只更新待显位置
            TrayIconEvent::Move { position, .. } => {
                let _ = tx_tray.send(HoverMsg::Move(position));
            }
            // 离开托盘：仅取消待显，真正隐藏交给看门狗去抖
            TrayIconEvent::Leave { .. } => {
                let _ = tx_tray.send(HoverMsg::Leave);
            }
            // 左键双击：淡出 + 打开主界面
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
        WebviewUrl::App("hover_card.html?v=h2".into()),
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
/// 直接查询窗口可见性，不依赖任何全局状态。
pub fn update_tray(app: &AppHandle, status: &DayStatus) {
    let cfg = sync::lock(&app.state::<crate::AppState>().config, "state.config").clone();
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
