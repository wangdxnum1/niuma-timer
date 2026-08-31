//! 打工人活动统计模块。
//!
//! 通过 Windows 全局低级钩子（WH_MOUSE_LL + WH_KEYBOARD_LL）统计：
//! - 鼠标：移动次数 / 累计移动像素 / 左键单击 / 双击 / 右键 / 滚轮(次数+格数) / 中键 / 侧键
//! - 键盘：总按键次数 + 高频按键 Top 榜（按虚拟键码细分）
//!
//! 设计要点：
//! - 钩子回调只做「原子 +1 / 极小锁临界区」，绝不阻塞（LL 钩子要求快速返回）；
//! - 常驻合并线程每 10 秒把原子累计值刷进当天的 24 个「小时桶」并落盘 SQLite；
//! - 数据落盘到 niuma.db 的 act_hourly / act_keys 表（WAL 模式，崩溃安全）；
//! - 启动时自动从 SQLite 加载当天数据恢复统计，程序/电脑重启后接着计数，不归零；
//! - 退出前（RunEvent::Exit）再 flush 一把，正常退出零丢失；
//! - 跨天自动切换到新日期行，历史日期自然沉淀为历史数据（SQL 按日期过滤即可）。
//!
//! 统计生效范围：程序运行期间（App 常驻托盘即持续统计）。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use chrono::{Local, Timelike};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_LBUTTONDOWN, WM_MBUTTONDOWN,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_XBUTTONDOWN, KBDLLHOOKSTRUCT, MSLLHOOKSTRUCT,
    MSG,
};

// ---------------------------------------------------------------------------
// 数据结构
// ---------------------------------------------------------------------------

/// 一小时内的活动累计。所有字段为 u64 计数。
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct HourBucket {
    /// 鼠标移动采样次数
    pub moves: u64,
    /// 累计移动距离（物理像素）
    pub pixels: u64,
    /// 左键按下次数（含双击的两次）
    pub left: u64,
    /// 双击次数（独立于 left 的附加识别）
    pub dbl: u64,
    /// 右键按下次数
    pub right: u64,
    /// 滚轮滚动事件次数
    pub wheel: u64,
    /// 滚轮滚动格数（WHEEL_DELTA=120 为 1 格）
    pub wheel_ticks: u64,
    /// 中键按下次数
    pub mid: u64,
    /// 侧键（XBUTTON）按下次数
    pub xbtn: u64,
    /// 键盘按键次数（已过滤按住自动重复）
    pub keys: u64,
}

impl HourBucket {
    /// 该小时「事件总数」：用于图表高度/活跃度排序（像素不计入，避免数值淹没）。
    fn total_events(&self) -> u64 {
        self.moves + self.left + self.dbl + self.right + self.wheel + self.mid + self.xbtn + self.keys
    }

    fn add(&mut self, o: &HourBucket) {
        self.moves += o.moves;
        self.pixels += o.pixels;
        self.left += o.left;
        self.dbl += o.dbl;
        self.right += o.right;
        self.wheel += o.wheel;
        self.wheel_ticks += o.wheel_ticks;
        self.mid += o.mid;
        self.xbtn += o.xbtn;
        self.keys += o.keys;
    }
}

/// 高频键条目（返回给前端展示）
#[derive(Debug, Clone, Serialize)]
pub struct KeyCount {
    pub key: String,
    pub count: u64,
}

/// 当日活动汇总（get_activity_summary 命令返回值）
#[derive(Debug, Clone, Serialize)]
pub struct ActivitySummary {
    /// 统计日期 "YYYY-MM-DD"
    pub date: String,
    /// 24 个小时桶（含当前小时未落盘的实时增量）
    pub hourly: Vec<HourBucket>,
    /// 当日汇总
    pub totals: HourBucket,
    /// 高频按键 Top 10
    pub top_keys: Vec<KeyCount>,
    /// 有活动的小时数（粗略「活跃时长」）
    pub active_hours: u64,
    /// 全局钩子是否安装成功
    pub hook_ok: bool,
}

// ---------------------------------------------------------------------------
// 内存状态
// ---------------------------------------------------------------------------

/// 钩子回调只更新这些原子计数器（零锁、极快）。
static C_MOVES: AtomicU64 = AtomicU64::new(0);
static C_PIXELS: AtomicU64 = AtomicU64::new(0);
static C_LEFT: AtomicU64 = AtomicU64::new(0);
static C_DBL: AtomicU64 = AtomicU64::new(0);
static C_RIGHT: AtomicU64 = AtomicU64::new(0);
static C_WHEEL: AtomicU64 = AtomicU64::new(0);
static C_WHEEL_TICKS: AtomicU64 = AtomicU64::new(0);
static C_MID: AtomicU64 = AtomicU64::new(0);
static C_XBTN: AtomicU64 = AtomicU64::new(0);
static C_KEYS: AtomicU64 = AtomicU64::new(0);

/// 钩子安装结果（供前端展示「统计是否生效」）
static HOOK_OK: AtomicBool = AtomicBool::new(false);
static HOOK_STARTED: AtomicBool = AtomicBool::new(false);

/// 活动监控总开关（设置页可切换）。关闭时钩子仍挂着（便于随时重开），
/// 但回调只刷新「最后输入时间」供挂机判定用，不再计数。
pub static ENABLED: AtomicBool = AtomicBool::new(true);

/// 运行时切换活动监控（立即生效）
pub fn set_enabled(v: bool) {
    ENABLED.store(v, Ordering::SeqCst);
}

/// 最近一次输入的时刻（毫秒时间戳）。供 app_usage 判断「是否挂机」：
/// 前台窗口属于白名单应用，但超过阈值无输入 → 视为挂机，不计使用时长。
static LAST_INPUT_MS: AtomicU64 = AtomicU64::new(0);

/// 当前毫秒时间戳（自 Unix 纪元）
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 钩子回调内调用：记录一次真实输入（原子 store，零阻塞）。
/// 鼠标任何事件 + 键盘按下都算。
fn note_input() {
    LAST_INPUT_MS.store(now_ms(), Ordering::Relaxed);
}

/// 查询最近一次输入时刻（毫秒）。app_usage 用它判断挂机阈值。
pub fn last_input_ms() -> u64 {
    LAST_INPUT_MS.load(Ordering::Relaxed)
}

/// 鼠标上次位置（首次移动只记录、不累计距离）
static LAST_POS: Mutex<Option<(i32, i32)>> = Mutex::new(None);
/// 左键上次按下 (time, x, y)，用于双击判定
static LAST_LEFT: Mutex<(u32, i32, i32)> = Mutex::new((0, 0, 0));

/// 键盘状态：按键明细增量 + 每个键上次按下时间（过滤 auto-repeat）
struct KeyState {
    /// 钩子回调产生的增量（合并线程消费后清空）
    pending: BTreeMap<u32, u64>,
    /// 每个键码上次按下时刻（毫秒，用于去重）
    last_time: BTreeMap<u32, u32>,
}
static KEY_STATE: Mutex<KeyState> = Mutex::new(KeyState {
    pending: BTreeMap::new(),
    last_time: BTreeMap::new(),
});

/// 当天累计状态（24 小时桶 + 当天按键全量明细）。
/// 仅用于从旧 JSON 迁移时的反序列化（db.rs），正常读写走 SQLite。
#[derive(Deserialize)]
pub(crate) struct DayState {
    pub(crate) date: String,
    pub(crate) hourly: Vec<HourBucket>,
    pub(crate) key_detail: BTreeMap<u32, u64>,
}

impl DayState {
    fn new(date: String) -> Self {
        Self {
            date,
            hourly: vec![HourBucket::default(); 24],
            key_detail: BTreeMap::new(),
        }
    }
}

/// 用 OnceLock：DayState 含 Vec 无法 const 初始化。
fn day() -> &'static Mutex<DayState> {
    static DAY: OnceLock<Mutex<DayState>> = OnceLock::new();
    DAY.get_or_init(|| {
        Mutex::new(DayState::new(Local::now().date_naive().format("%Y-%m-%d").to_string()))
    })
}

// ---------------------------------------------------------------------------
// 落盘（SQLite）
// ---------------------------------------------------------------------------

/// 把当天内存状态整体写入 SQLite：24 小时桶全量 UPSERT + 按键明细全量重写。
/// 单事务提交，WAL 模式下原子落盘，崩溃不会损坏。
fn save_day(d: &DayState) {
    let mut g = crate::db::conn().lock().unwrap();
    let tx = match g.transaction() {
        Ok(t) => t,
        Err(_) => return,
    };
    for (i, b) in d.hourly.iter().enumerate() {
        let _ = tx.execute(
            "INSERT OR REPLACE INTO act_hourly \
             (date, hour, moves, pixels, left, dbl, right, wheel, wheel_ticks, mid, xbtn, keys) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                d.date,
                i as i64,
                b.moves as i64,
                b.pixels as i64,
                b.left as i64,
                b.dbl as i64,
                b.right as i64,
                b.wheel as i64,
                b.wheel_ticks as i64,
                b.mid as i64,
                b.xbtn as i64,
                b.keys as i64
            ],
        );
    }
    // 按键明细是内存累积态，全量重写保证一致、无残留
    let _ = tx.execute("DELETE FROM act_keys WHERE date = ?1", params![d.date]);
    for (vk, cnt) in &d.key_detail {
        let _ = tx.execute(
            "INSERT OR REPLACE INTO act_keys (date, vk, count) VALUES (?1, ?2, ?3)",
            params![d.date, *vk as i64, *cnt as i64],
        );
    }
    let _ = tx.commit();
}

/// 从 SQLite 读取指定日期的活动统计（无数据则返回全零）。
/// 用 match 而非嵌套 if-let，避免 Statement 借用临时量存活到块尾的借用检查问题。
fn query_day_from_db(date: &str) -> (Vec<HourBucket>, BTreeMap<u32, u64>) {
    let mut hourly = vec![HourBucket::default(); 24];
    let mut keys = BTreeMap::new();
    let g = crate::db::conn().lock().unwrap();
    match g.prepare(
        "SELECT hour, moves, pixels, left, dbl, right, wheel, wheel_ticks, mid, xbtn, keys \
         FROM act_hourly WHERE date = ?1",
    ) {
        Ok(mut stmt) => match stmt.query_map(params![date], |r| {
            let hour: i64 = r.get(0)?;
            let b = HourBucket {
                moves: r.get(1)?,
                pixels: r.get(2)?,
                left: r.get(3)?,
                dbl: r.get(4)?,
                right: r.get(5)?,
                wheel: r.get(6)?,
                wheel_ticks: r.get(7)?,
                mid: r.get(8)?,
                xbtn: r.get(9)?,
                keys: r.get(10)?,
            };
            Ok((hour, b))
        }) {
            Ok(rows) => {
                for row in rows.flatten() {
                    if let Some(b) = hourly.get_mut(row.0 as usize) {
                        *b = row.1;
                    }
                }
            }
            Err(_) => {}
        },
        Err(_) => {}
    }
    match g.prepare("SELECT vk, count FROM act_keys WHERE date = ?1") {
        Ok(mut stmt) => match stmt.query_map(params![date], |r| {
            Ok((r.get::<_, i64>(0)? as u32, r.get::<_, i64>(1)? as u64))
        }) {
            Ok(rows) => {
                for row in rows.flatten() {
                    keys.insert(row.0, row.1);
                }
            }
            Err(_) => {}
        },
        Err(_) => {}
    }
    (hourly, keys)
}

/// 启动时恢复：从 SQLite 读取当天数据作为内存初始状态，
/// 程序/电脑重启后接着统计，不归零。
/// 注意：仅在启动早期（钩子与合并线程启动前）被调用，单线程执行，无锁竞争。
fn load_today() {
    let date = Local::now().date_naive().format("%Y-%m-%d").to_string();
    let (hourly, keys) = query_day_from_db(&date);
    let mut d = DayState::new(date);
    d.hourly = hourly;
    d.key_detail = keys;
    *day().lock().unwrap() = d;
}

/// 查询数据库（供未来历史浏览/聚合用）：读取指定日期的活动统计。
/// 目前前端只用「当天」，历史查询留作扩展。
#[allow(dead_code)]
pub fn load_day_from_db(date: &str) -> (Vec<HourBucket>, BTreeMap<u32, u64>) {
    query_day_from_db(date)
}

// ---------------------------------------------------------------------------
// 全局钩子（Windows 低级钩子，需跑在带消息循环的线程上）
// ---------------------------------------------------------------------------

/// 启动统计：恢复当天历史 → 安装钩子线程 + 常驻合并线程。幂等，仅首次生效。
pub fn start() {
    if HOOK_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    // 先恢复当天已落盘数据，再开始累加，重启不归零
    load_today();
    // 钩子线程：SetWindowsHookExW 后必须进入消息循环才能收到钩子消息
    std::thread::spawn(|| unsafe { hook_thread() });
    // 合并线程：每 10 秒把原子计数刷入当天小时桶并落盘
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(10));
        flush_pending();
    });
}

/// 程序退出前调用：把原子计数器里的最后增量刷进小时桶并落盘。
/// 正常退出（托盘退出 / 系统关闭）最多再丢 0 秒数据。
pub fn shutdown() {
    flush_pending();
}

unsafe fn hook_thread() {
    let m = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0);
    let k = SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_proc), None, 0);
    match (m, k) {
        (Ok(_m), Ok(_k)) => {
            HOOK_OK.store(true, Ordering::SeqCst);
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = UnhookWindowsHookEx(_m);
            let _ = UnhookWindowsHookEx(_k);
        }
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("[activity] 全局钩子安装失败: {e}");
            HOOK_OK.store(false, Ordering::SeqCst);
        }
    }
}

unsafe extern "system" fn mouse_proc(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if ncode >= 0 {
        // 任意鼠标事件都刷新「最后输入时间」（移动/按键/滚轮均算活跃）
        note_input();
        if !ENABLED.load(Ordering::Relaxed) {
            return CallNextHookEx(None, ncode, wparam, lparam);
        }
        let p = lparam.0 as *const MSLLHOOKSTRUCT;
        if !p.is_null() {
            let info = &*p;
            let msg = wparam.0 as u32;
            match msg {
                WM_MOUSEMOVE => {
                    C_MOVES.fetch_add(1, Ordering::Relaxed);
                    let mut last = LAST_POS.lock().unwrap();
                    match *last {
                        Some((lx, ly)) => {
                            let (dx, dy) = (info.pt.x - lx, info.pt.y - ly);
                            if dx != 0 || dy != 0 {
                                let dist = ((dx as i64 * dx as i64 + dy as i64 * dy as i64) as f64)
                                    .sqrt() as u64;
                                C_PIXELS.fetch_add(dist, Ordering::Relaxed);
                            }
                        }
                        None => {}
                    }
                    *last = Some((info.pt.x, info.pt.y));
                }
                WM_LBUTTONDOWN => {
                    C_LEFT.fetch_add(1, Ordering::Relaxed);
                    // 双击判定：间隔 < 系统双击时间 且 位移 ≤5px
                    let now = info.time;
                    let mut last = LAST_LEFT.lock().unwrap();
                    let dt = now.wrapping_sub(last.0);
                    let dx = (info.pt.x - last.1).abs();
                    let dy = (info.pt.y - last.2).abs();
                    if dt > 0 && dt < GetDoubleClickTime() && dx <= 5 && dy <= 5 {
                        C_DBL.fetch_add(1, Ordering::Relaxed);
                    }
                    *last = (now, info.pt.x, info.pt.y);
                }
                WM_RBUTTONDOWN => {
                    C_RIGHT.fetch_add(1, Ordering::Relaxed);
                }
                WM_MBUTTONDOWN => {
                    C_MID.fetch_add(1, Ordering::Relaxed);
                }
                WM_XBUTTONDOWN => {
                    C_XBTN.fetch_add(1, Ordering::Relaxed);
                }
                WM_MOUSEWHEEL => {
                    C_WHEEL.fetch_add(1, Ordering::Relaxed);
                    let ticks = ((info.mouseData as i32) >> 16).unsigned_abs() as u64;
                    C_WHEEL_TICKS.fetch_add(ticks.max(1), Ordering::Relaxed);
                }
                _ => {}
            }
        }
    }
    // LL 钩子必须继续传递，否则系统输入会中断
    CallNextHookEx(None, ncode, wparam, lparam)
}

unsafe extern "system" fn kb_proc(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if ncode >= 0 {
        // 键盘事件同样视为活跃输入
        note_input();
        if !ENABLED.load(Ordering::Relaxed) {
            return CallNextHookEx(None, ncode, wparam, lparam);
        }
        if wparam.0 as u32 == WM_KEYDOWN {
            let p = lparam.0 as *const KBDLLHOOKSTRUCT;
            if !p.is_null() {
                let info = &*p;
                let vk = info.vkCode;
                let now = info.time;
                let mut ks = KEY_STATE.lock().unwrap();
                let last = ks.last_time.get(&vk).copied().unwrap_or(0);
                // 按住自动重复间隔极短（~33ms），正常打字 ≥50ms，据此过滤重复计数
                if now.wrapping_sub(last) >= 50 {
                    C_KEYS.fetch_add(1, Ordering::Relaxed);
                    *ks.pending.entry(vk).or_insert(0) += 1;
                }
                *ks.last_time.entry(vk).or_insert(0) = now;
            }
        }
    }
    CallNextHookEx(None, ncode, wparam, lparam)
}

// ---------------------------------------------------------------------------
// 合并与查询
// ---------------------------------------------------------------------------

/// 把原子计数器 + 按键增量刷入当天小时桶，跨天则落盘旧文件并新建当天，随后写盘。
/// 由 10 秒合并线程与退出钩子调用（含落盘）。
fn flush_pending() {
    flush_pending_impl(true);
}

/// 查询前只刷内存、不落盘：供 summary 高频调用（前端每 2 秒轮询一次），
/// 避免每次查询都全量写库；持久化仍由 10 秒合并线程与退出钩子负责。
fn flush_pending_mem() {
    flush_pending_impl(false);
}

fn flush_pending_impl(persist: bool) {
    let now = Local::now();
    let date = now.date_naive().format("%Y-%m-%d").to_string();
    let hour = now.hour().min(23) as usize;

    let mut d = day().lock().unwrap();
    if d.date != date {
        save_day(&d);
        *d = DayState::new(date.clone());
    }
    let b = &mut d.hourly[hour];
    b.moves += C_MOVES.swap(0, Ordering::Relaxed);
    b.pixels += C_PIXELS.swap(0, Ordering::Relaxed);
    b.left += C_LEFT.swap(0, Ordering::Relaxed);
    b.dbl += C_DBL.swap(0, Ordering::Relaxed);
    b.right += C_RIGHT.swap(0, Ordering::Relaxed);
    b.wheel += C_WHEEL.swap(0, Ordering::Relaxed);
    b.wheel_ticks += C_WHEEL_TICKS.swap(0, Ordering::Relaxed);
    b.mid += C_MID.swap(0, Ordering::Relaxed);
    b.xbtn += C_XBTN.swap(0, Ordering::Relaxed);
    b.keys += C_KEYS.swap(0, Ordering::Relaxed);

    let mut ks = KEY_STATE.lock().unwrap();
    let pending = std::mem::take(&mut ks.pending);
    drop(ks);
    for (k, v) in pending {
        *d.key_detail.entry(k).or_insert(0) += v;
    }

    if persist {
        save_day(&d);
    }
}

/// 今日活动汇总（查询前先刷内存，保证包含最近几秒的实时增量）。
pub fn summary() -> ActivitySummary {
    flush_pending_mem();
    let d = day().lock().unwrap();
    let mut totals = HourBucket::default();
    let mut active_hours = 0u64;
    for b in &d.hourly {
        totals.add(b);
        if b.total_events() > 0 {
            active_hours += 1;
        }
    }
    // 高频键 Top 10
    let mut ranked: Vec<(u64, u32)> = d
        .key_detail
        .iter()
        .map(|(&k, &c)| (c, k))
        .collect();
    ranked.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    let top_keys: Vec<KeyCount> = ranked
        .into_iter()
        .take(10)
        .map(|(c, k)| KeyCount {
            key: vk_name(k),
            count: c,
        })
        .collect();

    ActivitySummary {
        date: d.date.clone(),
        hourly: d.hourly.clone(),
        totals,
        top_keys,
        active_hours,
        hook_ok: HOOK_OK.load(Ordering::SeqCst),
    }
}

// ---------------------------------------------------------------------------
// 虚拟键码 → 展示名
// ---------------------------------------------------------------------------

fn vk_name(vk: u32) -> String {
    match vk {
        0x08 => "退格".into(),
        0x09 => "Tab".into(),
        0x0D => "回车".into(),
        0x10 => "Shift".into(),
        0x11 => "Ctrl".into(),
        0x12 => "Alt".into(),
        0x13 => "Pause".into(),
        0x14 => "大写锁定".into(),
        0x1B => "Esc".into(),
        0x20 => "空格".into(),
        0x21 => "PgUp".into(),
        0x22 => "PgDn".into(),
        0x23 => "End".into(),
        0x24 => "Home".into(),
        0x25 => "←".into(),
        0x26 => "↑".into(),
        0x27 => "→".into(),
        0x28 => "↓".into(),
        0x2C => "截屏".into(),
        0x2D => "Insert".into(),
        0x2E => "Delete".into(),
        0x5B | 0x5C => "Win".into(),
        0x5D => "菜单键".into(),
        0x30..=0x39 => char::from(b'0' + (vk - 0x30) as u8).to_string(),
        0x41..=0x5A => char::from(b'A' + (vk - 0x41) as u8).to_string(),
        0x60..=0x69 => format!("小键盘{}", vk - 0x60),
        0x70..=0x87 => format!("F{}", vk - 0x70 + 1),
        0xBA => ";".into(),
        0xBB => "+".into(),
        0xBC => ",".into(),
        0xBD => "-".into(),
        0xBE => ".".into(),
        0xBF => "/".into(),
        0xC0 => "`".into(),
        0xDB => "[".into(),
        0xDC => "\\".into(),
        0xDD => "]".into(),
        0xDE => "'".into(),
        _ => format!("VK{:#X}", vk),
    }
}
