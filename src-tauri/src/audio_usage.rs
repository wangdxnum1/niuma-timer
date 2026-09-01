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
use std::time::Duration;

use chrono::{Local, Timelike};
use rusqlite::params;
use serde::Serialize;
use windows::core::{Interface, PWSTR};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator,
    MMDeviceEnumerator,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// 轮询间隔（秒）：每 5 秒结算一次
const POLL_INTERVAL_SECS: u64 = 5;

/// 发声判定阈值：会话峰值 > 该值视为正在播放（0.0~1.0 归一化）
const PEAK_THRESHOLD: f32 = 0.0;

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

/// PID → exe 完整路径（进程已退出/权限不足返回 None）
fn exe_path_of(pid: u32) -> Option<String> {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return None;
        };
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        let ok =
            QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut size);
        let _ = CloseHandle(h);
        if ok.is_ok() && size > 0 {
            let s = String::from_utf16_lossy(&buf[..size as usize]);
            return Some(s);
        }
    }
    None
}

/// 轮询一轮：枚举音频会话，返回正在发声的应用显示名（已跳过自身、已按应用去重）。
fn playing_apps() -> Vec<String> {
    unsafe {
        // COM 初始化（本线程每轮 init/uninit 成对；S_FALSE=已初始化也照常配对）
        // HRESULT::ok() 将负值视为错误，S_OK/S_FALSE 均视为成功
        if CoInitializeEx(None, COINIT_MULTITHREADED).ok().is_err() {
            return Vec::new();
        }
        let r = collect_playing();
        CoUninitialize();
        r
    }
}

unsafe fn collect_playing() -> Vec<String> {
    let Ok(enumerator) =
        CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
    else {
        return Vec::new();
    };
    // 默认渲染端点（扬声器/耳机）
    let Ok(device) = enumerator.GetDefaultAudioEndpoint(eRender, eConsole) else {
        return Vec::new();
    };
    let Ok(mgr) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
        return Vec::new();
    };
    let Ok(sessions) = mgr.GetSessionEnumerator() else {
        return Vec::new();
    };
    let Ok(count) = sessions.GetCount() else {
        return Vec::new();
    };

    let mut found = HashSet::new();
    for i in 0..count {
        let Ok(session) = sessions.GetSession(i) else {
            continue;
        };
        // 1) 会话峰值 > 阈值才可能计入（峰值取自上一个设备周期，周期为毫秒级，不影响 5 秒轮询）
        let Ok(meter) = session.cast::<IAudioMeterInformation>() else {
            continue;
        };
        let Ok(peak) = meter.GetPeakValue() else {
            continue;
        };
        if peak <= PEAK_THRESHOLD {
            continue;
        }
        // 2) 会话 → 进程 → exe 路径 → 软件名（复用 app_usage 的命名与图标逻辑）
        let Ok(ctrl2) = session.cast::<IAudioSessionControl2>() else {
            continue;
        };
        let Ok(pid) = ctrl2.GetProcessId() else {
            continue;
        };
        let Some(exe_path) = exe_path_of(pid) else {
            continue;
        };
        let exe_name = exe_path.rsplit('\\').next().unwrap_or("").to_lowercase();
        if exe_name == crate::app_usage::SELF_EXE {
            continue;
        }
        let display = crate::app_usage::display_name_of(&exe_path, &exe_name);
        // 不再于热轮询里同步提取图标：只记录「显示名→exe路径」，留给 summary() 懒提取
        playing_exe().lock().unwrap().insert(display.clone(), exe_path);
        found.insert(display);
    }
    found.into_iter().collect()
}

/// 结算线程：每 5 秒把「发声中的应用」各 +5 秒（按应用去重后）
fn tick_loop() {
    loop {
        std::thread::sleep(Duration::from_secs(POLL_INTERVAL_SECS));
        if !ENABLED.load(Ordering::Relaxed) {
            continue; // 已停用：跳过枚举与落盘，纯休眠
        }
        let apps = playing_apps();
        if apps.is_empty() {
            continue;
        }
        WATCH_OK.store(true, Ordering::SeqCst);
        let now_dt = Local::now();
        let date = now_dt.date_naive().format("%Y-%m-%d").to_string();
        let hour = now_dt.hour().min(23) as i64;
        let g = crate::db::conn().lock().unwrap();
        for app in apps {
            let _ = g.execute(
                "INSERT INTO audio_usage (date, app, seconds) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(date, app) DO UPDATE SET seconds = seconds + ?3",
                params![date, app, POLL_INTERVAL_SECS as i64],
            );
            let _ = g.execute(
                "INSERT INTO audio_usage_hourly (date, hour, app, seconds) VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(date, hour, app) DO UPDATE SET seconds = seconds + ?4",
                params![date, hour, app, POLL_INTERVAL_SECS as i64],
            );
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

/// 今日媒体播放汇总
pub fn summary() -> AudioUsageSummary {
    let date = Local::now().date_naive().format("%Y-%m-%d").to_string();
    let g = crate::db::conn().lock().unwrap();

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
                let icon = crate::app_usage::cached_icon(&app);
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

    let watch_ok = WATCH_OK.load(Ordering::SeqCst);
    AudioUsageSummary {
        date,
        apps,
        hourly,
        watch_ok,
    }
}
