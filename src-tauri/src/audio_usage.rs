//! 媒体播放时长统计模块（独立于「应用使用」，两套表互不混算）。
//!
//! 概念：统计「系统里正在发声的软件」的累计播放时长。
//! - 判定信号：Windows Core Audio 音频会话实时峰值（IAudioMeterInformation::GetPeakValue），
//!   峰值 > 0 = 该进程正在发声（前后台不限，无需维护白名单）；
//! - 覆盖场景：网易云/QQ音乐听歌、浏览器看视频、播客、游戏音效等一切有声软件；
//! - 不统计：无输出的挂机、后台驻留；系统声音会话（PID=0）天然被过滤；
//! - 结算：常驻线程每 5 秒枚举一次音频会话，发声中的应用**按应用去重**后各 +5 秒，
//!   写入独立表 audio_usage（天级）+ audio_usage_hourly（小时分布）。
//!
//! 与应用使用（app_usage）的关系：完全独立。app_usage 靠「前台窗口 + 输入」判定主动使用，
//! audio_usage 靠「音频输出」判定媒体播放，两者并列、各自明细。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::{Local, Timelike};
use rusqlite::params;
use serde::Serialize;

/// 会话枚举周期（秒）：每 5 秒重新枚举一次音频会话（含进程名解析，开销在大头）
const POLL_INTERVAL_SECS: u64 = 5;

/// 峰值采样间隔（毫秒）：枚举周期内按此节奏反复读各会话峰值。
///
/// 为什么不能像旧实现那样「一个枚举周期只读一次」：`GetPeakValue` 返回的是
/// **上一个设备周期**（典型 10ms 级）的峰值快照，不是自上次调用以来的累积值
/// （MSDN 原文：peak value is recorded over one device period and made available
/// during the subsequent device period）。5 秒读一次等于拿 10 毫秒的瞬时值代表
/// 5 秒——0.2 秒的提示音只有 4% 概率被采到，一旦命中就白送 5 秒。
/// 密采样后每个采样点只代表一个很短的真实间隔，短音不再被放大成长时长。
const SAMPLE_INTERVAL_MS: u64 = 200;

/// 发声判定阈值：会话峰值 > 该值视为正在播放（0.0~1.0 归一化）。
///
/// 取 0 是有意的：峰值是设备周期内的最大值，只要那个周期里有声音就会 > 0，
/// 真正的静音会话返回 0.0。抬阈值只会误杀小声播放（听歌音量低、远距离语音），
/// 却拦不住提示音——提示音恰恰是响的。抗提示音靠的是密采样，不是阈值。
const PEAK_THRESHOLD: f32 = 0.0;

/// 各应用「未满 1 秒」的播放余量（毫秒），跨枚举周期结转。
/// 密采样后单个采样点只有 200ms，不足 1 秒；不结转的话短促提示音会被反复抹零。
fn pending_ms() -> &'static Mutex<HashMap<String, u64>> {
    static MAP: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 播放中的「显示名 → exe 路径」映射：collect_playing 轮询时填充，
/// 供 summary() 在查询路径懒提取图标（不再于 5 秒热轮询里同步做 GDI+PNG）。
/// （HashMap::new 不是 const fn，不能用在 static 直接量里，故用 OnceLock 延迟初始化）
fn playing_exe() -> &'static Mutex<HashMap<String, String>> {
    static MAP: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

static WATCH_STARTED: AtomicBool = AtomicBool::new(false);
static WATCH_OK: AtomicBool = AtomicBool::new(false);

/// 媒体播放监控总开关（设置页可切换）。关闭时轮询线程只休眠，
/// 完全不碰 Core Audio API，零 CPU 开销。
pub static ENABLED: AtomicBool = AtomicBool::new(true);

/// 运行时切换媒体播放监控（立即生效）
pub fn set_enabled(v: bool) {
    ENABLED.store(v, Ordering::SeqCst);
}

/// 把本轮枚举到的音频会话解析成「应用显示名 → 峰值计」，供密集采样复用。
///
/// 返回值刻意保留**全部**会话（含此刻静音的）：会话可能在本轮采样途中才开始发声，
/// 枚举时按峰值过滤会把它们提前排除掉。进程名解析 / 软件名映射只在这里做一次
/// （每个枚举周期一次），采样期间不再碰任何进程 API。
///
/// 会话枚举本身（Core Audio / COM）已收口于 `win::audio_meters`，这里只做
/// 「进程 → 显示名」的业务映射，不接触任何 Win32 API。
fn resolve_meters(meters: Vec<crate::win::AudioMeter>) -> Vec<(String, crate::win::AudioMeter)> {
    let mut found: Vec<(String, crate::win::AudioMeter)> = Vec::new();
    for m in meters {
        // PID=0 是系统声音会话，没有对应的用户进程，无法归属到任何软件
        if m.pid == 0 {
            continue;
        }
        // 会话 → 进程 → exe 路径 → 软件名（复用 app_usage 的命名与图标逻辑）
        let Some(exe_path) = crate::win::process_exe_path(m.pid) else {
            continue;
        };
        let exe_name = exe_path.rsplit('\\').next().unwrap_or("").to_lowercase();
        if exe_name == crate::app_usage::SELF_EXE {
            continue;
        }
        let display = crate::app_usage::display_name_of(&exe_path, &exe_name);
        // 不再于热轮询里同步提取图标：只记录「显示名→exe路径」，留给 summary() 懒提取
        playing_exe().lock().unwrap().insert(display.clone(), exe_path);
        found.push((display, m));
    }
    found
}

/// 在一个枚举周期内密集采样各会话峰值，返回「应用 → 有声毫秒数」。
///
/// 同一应用有多个会话（如浏览器多标签）时按应用去重：任一会话有声即计为该应用
/// 有声，绝不叠加，否则开两个标签页播放时长就翻倍。
///
/// 记账单位用**真实经过时间**而非名义间隔：`sleep(200ms)` 的实际睡眠受系统计时器
/// 分辨率（默认 15.6ms）与调度影响，按名义值记会引入系统性偏差。
/// 但单次间隔要封顶——系统休眠/长时间卡顿醒来时 dt 可达数千秒，
/// 而休眠期间声卡并不输出声音，按名义间隔记才不会把休眠时长算成播放时长。
fn sample_round(meters: &[(String, crate::win::AudioMeter)]) -> HashMap<String, u64> {
    let mut acc: HashMap<String, u64> = HashMap::new();
    let deadline = Duration::from_secs(POLL_INTERVAL_SECS);
    let start = Instant::now();
    let mut last = start;
    while start.elapsed() < deadline {
        std::thread::sleep(Duration::from_millis(SAMPLE_INTERVAL_MS));
        if !ENABLED.load(Ordering::Relaxed) {
            break; // 采样途中被停用：立即收尾，已采到的部分照常记账
        }
        let now = Instant::now();
        let dt = ((now - last).as_millis() as u64).min(SAMPLE_INTERVAL_MS * 2);
        last = now;

        // 先判定再累加：保证同一应用有多个会话时本轮只记一次 dt
        let mut heard: HashSet<&str> = HashSet::new();
        for (app, meter) in meters {
            // 会话中途消失（应用关闭）时读峰会失败，按静音处理即可——
            // 下一轮枚举自会拿到最新的会话列表
            if let Some(peak) = meter.peak() {
                if peak > PEAK_THRESHOLD {
                    heard.insert(app.as_str());
                }
            }
        }
        for app in heard {
            *acc.entry(app.to_string()).or_insert(0) += dt;
        }
    }
    acc
}

/// 把本轮采样到的「各应用有声毫秒」记入数据库：满 1 秒才落盘，余量跨轮结转。
fn credit(played_ms: &HashMap<String, u64>) {
    let mut pending = pending_ms().lock().unwrap();
    let mut due: Vec<(String, i64)> = Vec::new();
    for (app, ms) in played_ms {
        let total = pending.entry(app.clone()).or_insert(0);
        *total += ms;
        let secs = *total / 1000;
        if secs > 0 {
            *total -= secs * 1000;
            due.push((app.clone(), secs as i64));
        }
    }
    drop(pending); // 先放锁再碰数据库
    if due.is_empty() {
        return;
    }
    let now_dt = Local::now();
    let date = now_dt.date_naive().format("%Y-%m-%d").to_string();
    let hour = now_dt.hour().min(23) as i64;
    let _ = crate::db::with_db(|g| -> rusqlite::Result<()> {
        for (app, secs) in due {
            g.execute(
                "INSERT INTO audio_usage (date, app, seconds) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(date, app) DO UPDATE SET seconds = seconds + ?3",
                params![date, app, secs],
            )?;
            g.execute(
                "INSERT INTO audio_usage_hourly (date, hour, app, seconds) VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(date, hour, app) DO UPDATE SET seconds = seconds + ?4",
                params![date, hour, app, secs],
            )?;
        }
        Ok(())
    });
}

/// 枚举 + 采样 + 落盘的主循环。
fn tick_loop() {
    // COM 在本线程只初始化一次：采样期间要跨整个枚举周期持有峰值计，
    // 不能每轮 init/uninit 成对——CoUninitialize 会把 COM 对象一起带走。
    // 守卫常驻到本线程结束，退出时自动反初始化。
    let Some(_com) = crate::win::ComGuard::init_multithreaded() else {
        eprintln!("[audio_usage] COM 初始化失败，媒体播放监控不可用");
        WATCH_OK.store(false, Ordering::SeqCst);
        return;
    };
    loop {
        if !ENABLED.load(Ordering::Relaxed) {
            // 已停用：跳过枚举与落盘，纯休眠，完全不碰 Core Audio API
            std::thread::sleep(Duration::from_secs(POLL_INTERVAL_SECS));
            continue;
        }
        match crate::win::audio_meters() {
            // Core Audio 不可用（无渲染设备 / COM 异常）：标记失效，前端据此提示。
            // 旧实现把「枚举失败」和「没有会话」混为一谈，WATCH_OK 一旦置 true 就不回退，
            // 拔掉音频设备后界面仍显示监控正常。
            None => {
                WATCH_OK.store(false, Ordering::SeqCst);
                std::thread::sleep(Duration::from_secs(POLL_INTERVAL_SECS));
            }
            // 枚举成功但没有会话：监控本身是好的，只是此刻没人发声
            Some(sessions) if sessions.is_empty() => {
                WATCH_OK.store(true, Ordering::SeqCst);
                std::thread::sleep(Duration::from_secs(POLL_INTERVAL_SECS));
            }
            // 枚举本身即计时器：sample_round 内的密集采样已经把这一轮的时间走完，
            // 不再额外 sleep，否则每轮都有一半时间处于「没在采样」的空窗。
            Some(sessions) => {
                WATCH_OK.store(true, Ordering::SeqCst);
                let meters = resolve_meters(sessions);
                if !meters.is_empty() {
                    let played = sample_round(&meters);
                    credit(&played);
                }
            }
        }
    }
}

/// 启动媒体播放监控线程。幂等，仅首次生效。
pub fn start() {
    if WATCH_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(tick_loop);
}

/// 程序退出前调用（5 秒粒度最多丢 5 秒，可接受，无需额外结算）。
pub fn shutdown() {}

// ---------------------------------------------------------------------------
// 查询（get_audio_usage_summary 命令）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct AudioUsageItem {
    pub app: String,
    pub seconds: i64,
    /// 应用图标（base64 PNG data URL），无图标为 None（前端显示首字占位）
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioUsageSummary {
    /// 统计日期 "YYYY-MM-DD"
    pub date: String,
    /// 今日各应用播放时长（按时长倒序）
    pub apps: Vec<AudioUsageItem>,
    /// 24 个小时桶：今日所有应用在该小时的合计播放秒数（未命中小时为 0）
    pub hourly: Vec<i64>,
    /// 音频监控是否生效
    pub watch_ok: bool,
}

/// 媒体播放汇总（默认今天，`date` 可指定任意历史日期）。
///
/// `known_icons`：前端已缓存过图标的应用名，命中的条目 `icon` 返回 `None`。
/// 与 `app_usage::summary` 同理——避免每 2 秒轮询反复回传整批图标 base64。
pub fn summary(known_icons: &[String], date: Option<&str>) -> AudioUsageSummary {
    let known: HashSet<&str> = known_icons.iter().map(|s| s.as_str()).collect();
    let date = match date {
        Some(d) => d.to_string(),
        None => Local::now().date_naive().format("%Y-%m-%d").to_string(),
    };
    let watch_ok = WATCH_OK.load(Ordering::SeqCst);
    // 查询失败时降级为空汇总（前端显示「暂无数据」），错误已由 with_db 记入 debug.log
    let (apps, hourly) = crate::db::with_db(|g| {
        let mut apps = Vec::new();
        if let Ok(mut stmt) = g.prepare(
            "SELECT app, seconds FROM audio_usage WHERE date = ?1 ORDER BY seconds DESC",
        ) {
            if let Ok(rows) = stmt.query_map(params![date], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            }) {
                for row in rows.flatten() {
                    let app = row.0;
                    // 懒提取：若尚未缓存图标，用轮询时记录的 exe 路径补提取（GDI+PNG 一次，幂等）
                    if let Some(exe) = playing_exe().lock().unwrap().get(&app).cloned() {
                        crate::app_usage::ensure_icon(&app, &exe);
                    }
                    let icon = if known.contains(app.as_str()) {
                        None // 前端已有，本次不再回传
                    } else {
                        crate::app_usage::cached_icon(&app)
                    };
                    apps.push(AudioUsageItem {
                        app,
                        seconds: row.1,
                        icon,
                    });
                }
            }
        }

        let mut hourly = vec![0i64; 24];
        if let Ok(mut stmt) = g.prepare(
            "SELECT hour, SUM(seconds) FROM audio_usage_hourly \
             WHERE date = ?1 GROUP BY hour ORDER BY hour",
        ) {
            if let Ok(rows) = stmt.query_map(params![date], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            }) {
                for row in rows.flatten() {
                    if let Some(b) = hourly.get_mut(row.0 as usize) {
                        *b = row.1;
                    }
                }
            }
        }
        Ok((apps, hourly))
    })
    .unwrap_or_else(|_| (Vec::new(), vec![0i64; 24]));

    AudioUsageSummary {
        date,
        apps,
        hourly,
        watch_ok,
    }
}
