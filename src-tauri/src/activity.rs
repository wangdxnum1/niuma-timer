//! 打工人活动统计模块。
//!
//! 通过 Windows Raw Input（`RegisterRawInputDevices` + message-only 窗口的 `WM_INPUT`）
//! 统计：
//! - 鼠标：移动次数 / 累计移动像素 / 左键单击 / 双击 / 右键 / 滚轮(次数+格数) / 中键 / 侧键
//! - 键盘：总按键次数 + 高频按键 Top 榜（按虚拟键码细分）
//!
//! 为什么用 Raw Input 而不是低级钩子（WH_MOUSE_LL / WH_KEYBOARD_LL）：
//! 低级钩子是「系统拦截」模型——每次按键系统先跨线程派发到钩子回调、等它返回才继续
//! 把按键投递给目标程序，路径本身就会引入输入延迟（尤其 IME 卡顿），瘦回调也救不了。
//! Raw Input 是「旁路投递」：系统把原始输入额外投递到本程序窗口的 `WM_INPUT`，
//! 主输入路径（键盘→IME→目标程序）完全不被阻塞，输入法零延迟；同时仍能逐键统计。
//!
//! 设计要点：
//! - `WM_INPUT` 回调只做「采集」：鼠标走原子 +1；键盘把 (vk, time) 打包进 SPSC 环形队列后
//!   立刻返回，开关判断 / 去重 / 计数 / 明细累加全部交给 keyq_worker 线程
//!   （回调里不留任何原子读改写——fetch_add 走 lock 指令，是输入路径上最贵的一类操作）；
//! - 挂机判定不复打点：直接问系统的 GetLastInputInfo，回调零额外开销，
//!   且覆盖面优于自造时钟（详见 last_input_ms 注释）；
//! - 关闭活动监控 = 真正注销 Raw Input 设备，而非「回调里不计数」：只要设备还注册着，
//!   每次输入仍会旁路投递到本窗口的 WndProc 并跑一遍判定。故停用即 RIDEV_REMOVE、
//!   启用即重注册（详见 ENABLED 与 raw_thread 注释）；
//! - 其余状态一律锁无关（原子 +1 / 节流时钟），绝不阻塞（Raw Input 走系统输入旁路，
//!   热路径任何锁/系统调用都会放大卡顿与鼠标漂移，故鼠标位置/双击一律走原子）；
//! - 常驻合并线程每 10 秒把原子累计值刷进当天的 24 个「小时桶」并落盘 SQLite；
//! - 数据落盘到 niuma.db 的 act_hourly / act_keys 表（WAL 模式，崩溃安全）；
//! - 启动时自动从 SQLite 加载当天数据恢复统计，程序/电脑重启后接着计数，不归零；
//! - 退出前（RunEvent::Exit）再 flush 一把，正常退出零丢失；
//! - 跨天自动切换到新日期行，历史日期自然沉淀为历史数据（SQL 按日期过滤即可）。
//!
//! 统计生效范围：程序运行期间（App 常驻托盘即持续统计）。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use chrono::{Local, Timelike};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::{
    RAWINPUT, RAWKEYBOARD, RAWMOUSE, RIM_TYPEKEYBOARD, RIM_TYPEMOUSE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetDoubleClickTime, GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, GetCursorPos, GetMessageTime, PostThreadMessageW, RI_KEY_BREAK,
    RI_MOUSE_BUTTON_4_DOWN, RI_MOUSE_BUTTON_5_DOWN, RI_MOUSE_HWHEEL, RI_MOUSE_LEFT_BUTTON_DOWN,
    RI_MOUSE_MIDDLE_BUTTON_DOWN, RI_MOUSE_RIGHT_BUTTON_DOWN, RI_MOUSE_WHEEL, WM_INPUT, WM_QUIT,
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
static RAW_OK: AtomicBool = AtomicBool::new(false);
static RAW_STARTED: AtomicBool = AtomicBool::new(false);

/// 钩子线程 ID，供停用时投递 `WM_QUIT` 唤醒阻塞在消息循环里的线程。
/// 0 = 线程尚未运行（此时根本没装钩子，无需唤醒）。
static RAW_TID: AtomicU32 = AtomicU32::new(0);

/// 活动监控总开关（设置页可切换）。
///
/// 关闭时**必须真正注销 Raw Input 设备**，而不能只是「回调里不计数」：
/// 只要设备还注册着，`WM_INPUT` 仍会旁路投递到本窗口的 WndProc，每次输入
/// 都要跑一遍采集判定（虽不阻塞主输入路径，仍是不必要的开销与潜在延迟源）。
/// 故停用 = `RIDEV_REMOVE` 注销，启用 = 重注册，由原始输入线程按本标志循环切换。
pub static ENABLED: AtomicBool = AtomicBool::new(true);

/// 运行时切换活动监控（立即生效）
pub fn set_enabled(v: bool) {
    let prev = ENABLED.swap(v, Ordering::SeqCst);
    if prev == v {
        return;
    }
    if !v {
        // 停用：唤醒原始输入线程使其退出消息循环，随后注销 Raw Input 设备 + 销毁窗口。
        // 若线程还没进入消息循环（TID 仍为 0），投递失败也无害——
        // 它在循环顶部就会看到 ENABLED=false，压根不会注册设备。
        request_stop();
    }
    // 启用：无需额外动作，原始输入线程最多 200ms 后醒来重新注册设备
}

/// 请求原始输入线程退出消息循环（投递 WM_QUIT）。
fn request_stop() {
    let tid = RAW_TID.load(Ordering::SeqCst);
    if tid == 0 {
        return;
    }
    // 失败（线程已退出 / 无消息队列）可安全忽略：线程结束必然已注销设备
    let _ = unsafe { PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0)) };
}

/// 当前毫秒时间戳（自 Unix 纪元）
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 查询最近一次「真实输入」的墙钟时刻（毫秒）。app_usage 用它判断挂机阈值：
/// 前台窗口属于白名单应用，但超过阈值无输入 → 视为挂机，不计使用时长。
///
/// 直接问系统要，而不是让钩子回调自己打点维护——Windows 内核本就为每个登录会话
/// 维护了最后输入时刻（`GetLastInputInfo`），覆盖面与精度都好过自造时钟：
/// 自造版有两个硬伤：① 计数器初值为 0，启动后恒判挂机，要攒够 500 次输入才正常；
/// ② 为避免在输入路径上读时钟，每 500 次事件才刷新一次，慢速打字时时间戳可陈旧
/// 数百秒、直接越过 5 分钟挂机阈值，把真实使用当成挂机丢掉。
/// 系统版还顺带覆盖了钩子看不到的输入（部分全屏程序的原始输入），且钩子回调里
/// 不再有任何打点开销。
///
/// 调用频率极低（仅 app_usage 每 10 秒结算一次），系统调用开销无关紧要。
pub fn last_input_ms() -> u64 {
    let mut lii = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // 失败极罕见；退化成「此刻刚有输入」——宁可多记一段，也别把真实使用判成挂机
    if !unsafe { GetLastInputInfo(&mut lii) }.as_bool() {
        return now_ms();
    }
    // dwTime 与 GetTickCount 同为 u32 毫秒计数，约 49.7 天回绕一次；
    // wrapping_sub 保证跨回绕点依然算得出正确差值
    let idle_ms = unsafe { GetTickCount() }.wrapping_sub(lii.dwTime) as u64;
    now_ms().saturating_sub(idle_ms)
}

/// 鼠标上次位置（首次移动只记录、不累计距离）。
/// 用两个原子取代 Mutex：钩子回调每收到 WM_MOUSEMOVE 都会读/写，锁无关才能避免
/// 在系统输入路径上抢锁带来的卡顿与鼠标漂移。
static LAST_X: AtomicI32 = AtomicI32::new(i32::MIN);
static LAST_Y: AtomicI32 = AtomicI32::new(i32::MIN);

/// 左键上次按下 (time, x, y)，用于双击判定。同样锁无关（原子三段），按钮按下频率
/// 远低于移动，开销更可忽略，但避免在输入路径上留任何锁。
static LAST_LBTN_TIME: AtomicU32 = AtomicU32::new(0);
static LAST_LBTN_X: AtomicI32 = AtomicI32::new(0);
static LAST_LBTN_Y: AtomicI32 = AtomicI32::new(0);

/// 键盘状态：每键码(0..=255)一个原子槽，避免 BTreeMap+Mutex 在每次按键时抢锁。
/// - KEY_LAST_TIME[vk]：该键上次按下时刻（毫秒，过滤自动重复），仅钩子回调写；
/// - KEY_PENDING[vk]：该键待合并的按键次数，钩子回调 +1、合并线程每 10s swap(0) 取走。
/// 用 OnceLock 延迟初始化定长原子数组（vkCode 范围 1..=254，256 足够覆盖）。
fn key_last_time() -> &'static [AtomicU32; 256] {
    static ARR: OnceLock<[AtomicU32; 256]> = OnceLock::new();
    ARR.get_or_init(|| std::array::from_fn(|_| AtomicU32::new(0)))
}
fn key_pending() -> &'static [AtomicU64; 256] {
    static ARR: OnceLock<[AtomicU64; 256]> = OnceLock::new();
    ARR.get_or_init(|| std::array::from_fn(|_| AtomicU64::new(0)))
}

// ---------------------------------------------------------------------------
// 键盘事件队列：回调只采集入队，去重与计数交给工作线程
// ---------------------------------------------------------------------------

/// 队列容量（2 的幂，便于用掩码取模，省一次除法）。1024 个按键增量对统计用途
/// 绰绰有余；工作线程每 20ms 排空一轮，正常打字速率下不可能堆积到溢出。
const KEYQ_CAP: usize = 1024;
const KEYQ_MASK: usize = KEYQ_CAP - 1;

/// 工作线程排空间隔（毫秒）。按键增量最迟 20ms 后进入统计，远小于 10 秒合并周期。
const KEYQ_DRAIN_MS: u64 = 20;

/// 单元素打包：低 8 位放虚拟键码，高 32 位放毫秒时间戳，一个 u64 装下，
/// 入队只需一次原子写（回调里最便宜的形态）。
#[inline(always)]
fn pack_key(vk: u32, time: u32) -> u64 {
    ((time as u64) << 8) | ((vk & 0xff) as u64)
}
#[inline(always)]
fn unpack_key(v: u64) -> (u32, u32) {
    ((v & 0xff) as u32, (v >> 8) as u32)
}

/// SPSC 环形队列：生产者只有钩子线程、消费者只有 keyq_worker，故无需加锁。
/// 用 OnceLock 延迟初始化 1024 长度的原子数组（AtomicU64 非 Copy，
/// 不能用数组重复表达式直接构造，与上面 key_last_time / key_pending 同一手法）。
static KEYQ: OnceLock<[AtomicU64; KEYQ_CAP]> = OnceLock::new();
fn keyq() -> &'static [AtomicU64; KEYQ_CAP] {
    KEYQ.get_or_init(|| std::array::from_fn(|_| AtomicU64::new(0)))
}
static KEYQ_HEAD: AtomicUsize = AtomicUsize::new(0);
static KEYQ_TAIL: AtomicUsize = AtomicUsize::new(0);

/// 入队（钩子回调调用）：队列满时直接丢弃——统计用途宁可丢数据，
/// 也绝不让输入路径等待或重试。
fn keyq_push(vk: u32, time: u32) {
    let h = KEYQ_HEAD.load(Ordering::Relaxed);
    let t = KEYQ_TAIL.load(Ordering::Acquire);
    if h.wrapping_sub(t) >= KEYQ_CAP {
        return;
    }
    keyq()[h & KEYQ_MASK].store(pack_key(vk, time), Ordering::Relaxed);
    KEYQ_HEAD.store(h.wrapping_add(1), Ordering::Release);
}

/// 出队（仅 keyq_worker 调用）：SPSC 要求单一消费者，故用 load/store 而非 CAS。
fn keyq_pop() -> Option<(u32, u32)> {
    let t = KEYQ_TAIL.load(Ordering::Relaxed);
    if t == KEYQ_HEAD.load(Ordering::Acquire) {
        return None;
    }
    let v = keyq()[t & KEYQ_MASK].load(Ordering::Relaxed);
    KEYQ_TAIL.store(t.wrapping_add(1), Ordering::Release);
    Some(unpack_key(v))
}

/// 键盘事件工作线程：消费队列里的按键增量，做开关判断 / 去重 / 计数 / 明细累加。
///
/// 原来这些逻辑内联在低级钩子的 kb_proc 里（每次按键都要跑三次原子 fetch_add：
/// INPUT_SEQ 打点、C_KEYS、key_pending[vk]，全部走 lock 指令），现在全部移到这里；
/// Raw Input 的键盘分支（on_raw_keyboard）同样只做采集、把数据丢进队列后即返回。
/// 回调已把数据放好，这里慢一点也完全不影响输入路径。
/// 注：输入活跃度打点已彻底取消，挂机判定改由 last_input_ms() 问系统要。
fn keyq_worker() {
    loop {
        // 批量排空：把本轮积累的按键一次处理完
        while let Some((vk, time)) = keyq_pop() {
            if !ENABLED.load(Ordering::Relaxed) {
                continue;
            }
            let vk = vk as usize;
            if vk < 256 {
                let last = key_last_time()[vk].load(Ordering::Relaxed);
                // 按住自动重复间隔极短（~33ms），正常打字 ≥50ms，据此过滤重复计数
                if time.wrapping_sub(last) >= 50 {
                    C_KEYS.fetch_add(1, Ordering::Relaxed);
                    key_pending()[vk].fetch_add(1, Ordering::Relaxed);
                }
                key_last_time()[vk].store(time, Ordering::Relaxed);
            } else {
                C_KEYS.fetch_add(1, Ordering::Relaxed);
            }
        }
        std::thread::sleep(Duration::from_millis(KEYQ_DRAIN_MS));
    }
}

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
// Raw Input（message-only 窗口的 WM_INPUT，需跑在带消息循环的线程上）
// ---------------------------------------------------------------------------

/// 启动统计：恢复当天历史 → 拉起原始输入线程 + 常驻合并线程。幂等，仅首次生效。
pub fn start() {
    if RAW_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    // 先恢复当天已落盘数据，再开始累加，重启不归零
    load_today();
    // 键盘事件工作线程：消费 WM_INPUT 键盘分支入队的按键增量（去重 / 计数 / 明细累加）
    std::thread::spawn(keyq_worker);
    // 原始输入线程：注册 Raw Input 后必须进入消息循环才能收到 WM_INPUT
    std::thread::spawn(|| unsafe { raw_thread() });
    // 合并线程：每 10 秒把原子计数刷入当天小时桶并落盘
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(10));
        flush_pending();
    });
}

/// 程序退出前调用：把原子计数器里的最后增量刷进小时桶并落盘。
/// 正常退出（托盘退出 / 系统关闭）最多再丢 0 秒数据。
pub fn shutdown() {
    // 先注销 Raw Input：退出流程里不再接收输入，队列随之停止增长
    request_stop();
    // 键盘增量走队列：给工作线程一点时间排空再落盘，否则最后 ≤20ms 的
    // 按键会留在队列里被丢掉（旧实现在回调里同步计数，不存在这个窗口）。
    std::thread::sleep(Duration::from_millis(KEYQ_DRAIN_MS * 2));
    flush_pending();
}

/// 原始输入线程主循环：按 ENABLED 开关循环执行「建窗口+注册 Raw Input → 消息循环 → 注销」。
///
/// 线程本身常驻不退出（避免反复 spawn 带来的竞态：新线程还没跑到注册设备，
/// 停用请求就已经发出），只在停用期间空转；Raw Input 设备严格随开关注册/注销——
/// 停用期间系统输入旁路不再投递到本窗口，输入零额外开销。
unsafe fn raw_thread() {
    // 记录线程 ID：set_enabled(false) / shutdown 靠它投递 WM_QUIT 唤醒下面的消息循环
    RAW_TID.store(GetCurrentThreadId(), Ordering::SeqCst);
    let class = raw_wnd_class();
    loop {
        if !ENABLED.load(Ordering::SeqCst) {
            // 已停用：此时设备必然是注销状态（上一轮末尾已调用 RIDEV_REMOVE），
            // 空转等待重新启用。开关切换是手动低频操作，200ms 轮询可忽略。
            std::thread::sleep(Duration::from_millis(200));
            continue;
        }
        // 建 message-only 窗口 + 注册 Raw Input（键+鼠，RIDEV_INPUTSINK）：
        // 即使本窗口不是前台窗口也能收到全局输入；绝不加 RIDEV_NOLEGACY，
        // 否则会禁用别的程序的 WM_KEYDOWN，变成劫持键盘。
        let hwnd = match crate::win::create_message_window(&class, Some(raw_wndproc)) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("[activity] Raw Input 窗口创建失败: {e}");
                RAW_OK.store(false, Ordering::SeqCst);
                return;
            }
        };
        if let Err(e) = crate::win::register_raw_input(hwnd) {
            eprintln!("[activity] Raw Input 注册失败: {e}");
            let _ = crate::win::destroy_message_window(hwnd);
            RAW_OK.store(false, Ordering::SeqCst);
            return;
        }
        RAW_OK.store(true, Ordering::SeqCst);
        // 阻塞直到本线程收到 WM_QUIT（由 set_enabled(false) / shutdown 投递）。
        // 这是 Raw Input 的硬性要求：WM_INPUT 靠线程消息队列驱动，必须有消息循环。
        // 边界：200ms 内快速「关→开」时，队列里可能残留一个多余的 WM_QUIT，
        // 会在下一轮注册后被立即取出——只多一次注册/注销循环，随后自愈，无害。
        crate::win::run_message_loop();
        // 收到 WM_QUIT：注销设备 + 销毁窗口，系统输入旁路不再有本模块。
        let _ = crate::win::unregister_raw_input(hwnd);
        let _ = crate::win::destroy_message_window(hwnd);
        RAW_OK.store(false, Ordering::SeqCst);
    }
}

/// 原始输入窗口类名（null 结尾的 UTF-16）。本进程内唯一，窗口类只注册一次。
fn raw_wnd_class() -> Vec<u16> {
    "NiumaRawInputWnd\0".encode_utf16().collect()
}

/// 把 `WM_INPUT` 的原始鼠标数据解析为统计。
///
/// 位置一律取自 `GetCursorPos`：Raw Input 的 `RAWMOUSE` 只给相对位移（且是设备 mickey，
/// 非屏幕像素），而双击判定需要屏幕绝对坐标；`GetCursorPos` 与旧 `MSLLHOOKSTRUCT.pt`
/// 同源（系统光标位置），既准又统一。移动距离 = 相邻两次 `WM_INPUT` 间的光标位移，
/// 与旧实现语义一致。
fn on_raw_mouse(m: &RAWMOUSE) {
    let bf = unsafe { m.Anonymous.Anonymous.usButtonFlags };
    // 移动：只要有相对/绝对移动标志或位移非零，就记一次 moves 并累加光标位移像素。
    // 绝对模式（触控板/笔）下 lLastX/Y 为绝对坐标，仍用 GetCursorPos 算位移，跨设备一致。
    let moved = m.usFlags.0 == 0 || m.usFlags.0 == 1; // MOVE_RELATIVE(0) / MOVE_ABSOLUTE(1)
    if moved || m.lLastX != 0 || m.lLastY != 0 {
        let mut pt = POINT { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut pt).is_ok() } {
            let (x, y) = (pt.x, pt.y);
            C_MOVES.fetch_add(1, Ordering::Relaxed);
            let (px, py) = (LAST_X.load(Ordering::Relaxed), LAST_Y.load(Ordering::Relaxed));
            if px != i32::MIN && py != i32::MIN {
                let (dx, dy) = (x - px, y - py);
                if dx != 0 || dy != 0 {
                    let dist = ((dx as i64 * dx as i64 + dy as i64 * dy as i64) as f64).sqrt() as u64;
                    C_PIXELS.fetch_add(dist, Ordering::Relaxed);
                }
            }
            LAST_X.store(x, Ordering::Relaxed);
            LAST_Y.store(y, Ordering::Relaxed);
        }
    }
    // 按键（usButtonFlags 的 *_DOWN 位各自独立，可同时出现，故分别判断）
    if bf & RI_MOUSE_LEFT_BUTTON_DOWN as u16 != 0 {
        C_LEFT.fetch_add(1, Ordering::Relaxed);
        // 双击判定：间隔 < 系统双击时间 且 位移 ≤5px（位置取自真实光标）
        let mut pt = POINT { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut pt).is_ok() } {
            let now = GetMessageTime() as u32;
            let (lt, lx, ly) = (
                LAST_LBTN_TIME.load(Ordering::Relaxed),
                LAST_LBTN_X.load(Ordering::Relaxed),
                LAST_LBTN_Y.load(Ordering::Relaxed),
            );
            let dt = now.wrapping_sub(lt);
            let (dx, dy) = ((pt.x - lx).abs(), (pt.y - ly).abs());
            if dt > 0 && dt < GetDoubleClickTime() && dx <= 5 && dy <= 5 {
                C_DBL.fetch_add(1, Ordering::Relaxed);
            }
            LAST_LBTN_TIME.store(now, Ordering::Relaxed);
            LAST_LBTN_X.store(pt.x, Ordering::Relaxed);
            LAST_LBTN_Y.store(pt.y, Ordering::Relaxed);
        }
    }
    if bf & RI_MOUSE_RIGHT_BUTTON_DOWN as u16 != 0 {
        C_RIGHT.fetch_add(1, Ordering::Relaxed);
    }
    if bf & RI_MOUSE_MIDDLE_BUTTON_DOWN as u16 != 0 {
        C_MID.fetch_add(1, Ordering::Relaxed);
    }
    if bf & RI_MOUSE_BUTTON_4_DOWN as u16 != 0 || bf & RI_MOUSE_BUTTON_5_DOWN as u16 != 0 {
        C_XBTN.fetch_add(1, Ordering::Relaxed);
    }
    if bf & RI_MOUSE_WHEEL as u16 != 0 || bf & RI_MOUSE_HWHEEL as u16 != 0 {
        C_WHEEL.fetch_add(1, Ordering::Relaxed);
        // 滚轮增量在 usButtonData（i16，符号表示方向，绝对值通常为 120 的整数倍）。
        // 与旧实现一致：直接累加绝对值（每格 120），前端按需 /120 展示。
        let data = unsafe { m.Anonymous.Anonymous.usButtonData } as i16;
        C_WHEEL_TICKS.fetch_add(data.unsigned_abs() as u64, Ordering::Relaxed);
    }
}

/// 把 `WM_INPUT` 的原始键盘数据解析为统计。
///
/// `RAWKEYBOARD.Flags` 的 `RI_KEY_BREAK` 位表示「键抬起」；无此位即「按下」。
/// 只做采集：把虚拟键码 + 消息时间打包入 SPSC 队列，去重（50ms 间隔自动重复）/
/// 计数 / 明细累加交给 keyq_worker（与旧钩子同分工）。
/// `GetMessageTime()` 与旧 `KBDLLHOOKSTRUCT.time` 同域（GetTickCount 毫秒），等价。
fn on_raw_keyboard(k: &RAWKEYBOARD) {
    if k.Flags & RI_KEY_BREAK as u16 != 0 {
        return; // 键抬起：不计
    }
    keyq_push(k.VKey as u32, GetMessageTime() as u32);
}

/// Raw Input 窗口过程：仅处理 `WM_INPUT`，其余消息交 `DefWindowProcW`。
///
/// 在 raw_thread 的消息循环里被 `DispatchMessageW` 调用。锁屏离开 / 停用时直接放行，
/// 不污染活动数据（与旧钩子回调同语义）。`WM_INPUT` 的 `lParam` 是 `HRAWINPUT`，
/// 经 `win::read_raw_input` 取回原始数据后按键盘 / 鼠标分流。
unsafe extern "system" fn raw_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_INPUT {
        if !crate::lock_monitor::is_away() && ENABLED.load(Ordering::Relaxed) {
            if let Some(buf) = crate::win::read_raw_input(lparam) {
                if buf.len() >= std::mem::size_of::<RAWINPUT>() {
                    // RAWINPUT 必存在；按其 dwType 分流键盘/鼠标
                    let raw = &*(buf.as_ptr() as *const RAWINPUT);
                    match raw.header.dwType {
                        t if t == RIM_TYPEMOUSE.0 => {
                            let m = unsafe { raw.data.mouse };
                            on_raw_mouse(&m);
                        }
                        t if t == RIM_TYPEKEYBOARD.0 => {
                            let k = unsafe { raw.data.keyboard };
                            on_raw_keyboard(&k);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
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

    // 取走键盘增量：遍历 256 个原子键槽，swap(0) 收归到 key_detail（vkCode 即数组下标）。
    // 锁无关：合并线程不再与钩子回调在每次按键时抢 KEY_STATE 锁，消除输入卡顿的并发来源。
    for (vk, slot) in key_pending().iter().enumerate() {
        let v = slot.swap(0, Ordering::Relaxed);
        if v > 0 {
            *d.key_detail.entry(vk as u32).or_insert(0) += v;
        }
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
        hook_ok: RAW_OK.load(Ordering::SeqCst),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `last_input_ms` 必须返回**墙钟毫秒**，且落在合理区间内。
    ///
    /// 这条断言主要兜两类实现错误：① 忘了设 `LASTINPUTINFO::cbSize` 导致 API 失败
    /// （会退化成返回 now_ms()，仍应通过）；② 误把 u32 tick count 当墙钟返回
    /// （tick count 上限约 4.29e9，远小于 1.7e12 这条下限，必被抓出）。
    #[test]
    fn last_input_ms_is_sane_wall_clock() {
        let now = now_ms();
        let t = last_input_ms();
        // 1_700_000_000_000 ms ≈ 2023-11，墙钟毫秒必然大于它
        assert!(t > 1_700_000_000_000, "返回值不像墙钟毫秒: {t}");
        assert!(t <= now + 1_000, "最后输入时刻晚于当前时刻: {t} > {now}");
        // 上限取 u32 tick count 的回绕周期 49.7 天，留足余量
        assert!(now.saturating_sub(t) <= 50 * 24 * 3600 * 1000);
    }
}
