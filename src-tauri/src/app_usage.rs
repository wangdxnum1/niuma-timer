//! 应用使用时长统计模块。
//!
//! 统计「所有前台应用被真实使用的时间」：
//! - 前台窗口识别：`SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` 事件驱动，
//!   窗口切换瞬间回调拿到 hwnd → PID → exe 路径；
//! - 软件名解析：已知映射表（微信/钉钉）→ exe 版本信息 FileDescription → 进程名兜底；
//! - 图标：`ExtractIconExW` 提取 exe 图标 → 内存 DIB → PNG → base64 data URL，
//!   内存 + 磁盘（config_dir/icons/）双缓存，只提取一次；
//! - 活跃判定：复用 activity 模块的「最后输入时间戳」，超过 5 分钟无输入视为挂机，
//!   挂机时段不计入使用时长；
//! - 结算：常驻线程每 10 秒按「距上次结算的真实时间差」累加，写入 SQLite 两张表
//!   （app_usage 天级汇总 + app_usage_hourly 小时分布）。
//!
//! 统计生效范围：程序运行期间（App 常驻托盘即持续统计），跳过自身进程。

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::ffi::c_void;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use chrono::{Local, Timelike};
use rusqlite::params;
use serde::Serialize;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetObjectW, SelectObject,
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Shell::ExtractIconExW;
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, DrawIconEx, GetForegroundWindow, GetIconInfo, GetWindowThreadProcessId, DI_NORMAL,
    EVENT_SYSTEM_FOREGROUND, HICON, ICONINFO, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};

/// 挂机阈值：距最后一次真实输入超过该时长（毫秒）→ 前台窗口不算使用中
const IDLE_THRESHOLD_MS: u64 = 5 * 60 * 1000;

/// 已知软件映射：(进程名小写, 用户展示名)。
///
/// 设计原则：把「程序识别名（exe 文件名）」与「用户展示名」解耦——
/// 命中映射表直接展示中文常用名，避免回退到 exe 版本资源里的英文
/// FileDescription（如 "NetEase Cloud Music"）让普通用户看不懂。
/// 国内软件给中文名；Chrome/Edge/Spotify 等大众认知度高的英文名保留原名。
/// 多收录无害（按 exe 名精确匹配，不会误映射），新增软件往这里加即可。
const KNOWN_MAP: &[(&str, &str)] = &[
    // ---- 通讯 / 办公 ----
    ("wechat.exe", "微信"),
    ("weixin.exe", "微信"),
    ("wechatappex.exe", "微信"),
    ("dingtalk.exe", "钉钉"),
    ("wemeetapp.exe", "腾讯会议"),
    ("feishu.exe", "飞书"),
    ("wecom.exe", "企业微信"),
    ("wps.exe", "WPS Office"),
    ("winword.exe", "Word"),
    ("excel.exe", "Excel"),
    ("powerpnt.exe", "PowerPoint"),
    ("onenote.exe", "OneNote"),
    ("notepad.exe", "记事本"),
    ("code.exe", "VS Code"),
    // ---- 音乐 / 视频 / 直播 ----
    ("cloudmusic.exe", "网易云音乐"),
    ("qqmusic.exe", "QQ音乐"),
    ("kugou.exe", "酷狗音乐"),
    ("bilibili.exe", "哔哩哔哩"),
    ("iqiyi.exe", "爱奇艺"),
    ("qqlive.exe", "腾讯视频"),
    ("youku.exe", "优酷"),
    ("douyin.exe", "抖音"),
    ("kuaishou.exe", "快手"),
    ("potplayermini64.exe", "PotPlayer"),
    ("vlc.exe", "VLC"),
    ("wmplayer.exe", "Windows Media Player"),
    ("spotify.exe", "Spotify"),
    // ---- 下载 / 网盘 ----
    ("thunder.exe", "迅雷"),
    ("baidunetdisk.exe", "百度网盘"),
    // ---- 浏览器（大众认知度高，保留英文名）----
    ("chrome.exe", "Google Chrome"),
    ("msedge.exe", "Microsoft Edge"),
    ("firefox.exe", "Firefox"),
    // ---- 游戏 / 平台 ----
    ("steam.exe", "Steam"),
    ("leagueclient.exe", "英雄联盟"),
    // ---- 工具 ----
    ("everything.exe", "Everything"),
    ("7zfm.exe", "7-Zip"),
];

/// 历史数据归一化用的「旧显示名(小写) → 中文名」别名表。
/// 早期版本库里存的是 exe 版本资源的英文 FileDescription（如 "NetEase Cloud Music"），
/// 映射表按 exe 名匹配无法反查它们，故单独维护一份别名表供 db::normalize_app_names 迁移。
const LEGACY_DISPLAY_NAMES: &[(&str, &str)] = &[
    ("netease cloud music", "网易云音乐"),
    ("qq music", "QQ音乐"),
    ("kugou", "酷狗音乐"),
    ("tencent meeting", "腾讯会议"),
    ("bilibili", "哔哩哔哩"),
    ("iqiyi", "爱奇艺"),
    ("qqlive", "腾讯视频"),
    ("youku", "优酷"),
    ("douyin", "抖音"),
    ("kuaishou", "快手"),
    ("thunder", "迅雷"),
    ("baidunetdisk", "百度网盘"),
    ("dingtalk", "钉钉"),
    ("wechat", "微信"),
    ("feishu", "飞书"),
    ("wps office", "WPS Office"),
    ("potplayer", "PotPlayer"),
    ("vlc media player", "VLC"),
    ("windows media player", "Windows Media Player"),
    ("spotify", "Spotify"),
    ("google chrome", "Google Chrome"),
    ("microsoft edge", "Microsoft Edge"),
    ("firefox", "Firefox"),
    ("steam", "Steam"),
    ("everything", "Everything"),
];

/// 旧显示名 → 中文名（大小写不敏感），供历史数据迁移使用。
pub(crate) fn map_legacy_display(app: &str) -> Option<String> {
    let key = app.to_lowercase();
    LEGACY_DISPLAY_NAMES
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.to_string())
}

/// 自身进程名：前台窗口是自己时不统计
pub(crate) const SELF_EXE: &str = "niuma-timer.exe";

/// 当前前台命中的应用
#[derive(Clone)]
struct CurApp {
    /// 显示名（软件名，如「微信」「钉钉」）
    display: String,
}

static CUR_APP: Mutex<Option<CurApp>> = Mutex::new(None);

/// 上次结算时刻（毫秒），用于按真实时间差累加
static LAST_TICK_MS: AtomicU64 = AtomicU64::new(0);

static WATCH_STARTED: AtomicBool = AtomicBool::new(false);
static WATCH_OK: AtomicBool = AtomicBool::new(false);

/// 应用使用监控总开关（设置页可切换）。关闭时前台切换事件被忽略、不累计时长。
pub static ENABLED: AtomicBool = AtomicBool::new(true);

/// 运行时切换应用使用监控（立即生效）
pub fn set_enabled(v: bool) {
    ENABLED.store(v, Ordering::SeqCst);
}

/// 重新开启监控时调用：把当前前台窗口立即纳入统计，无需等下次窗口切换
pub fn refresh_foreground() {
    unsafe {
        let fg = GetForegroundWindow();
        if !fg.is_invalid() {
            update_cur_app(fg);
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// 软件名解析
// ---------------------------------------------------------------------------

/// 进程 → 软件名：已知映射表 → FileDescription → 去扩展名的进程名兜底
/// （pub(crate) 供 audio_usage 媒体播放模块复用同一套命名逻辑）
pub(crate) fn display_name_of(exe_path: &str, exe_name_lower: &str) -> String {
    for (exe, display) in KNOWN_MAP {
        if exe_name_lower == *exe {
            return display.to_string();
        }
    }
    if let Some(fd) = file_description(exe_path) {
        let fd = fd.trim();
        if !fd.is_empty() {
            return fd.to_string();
        }
    }
    exe_name_lower
        .rsplit('.')
        .next()
        .unwrap_or(exe_name_lower)
        .to_string()
}

/// 读 exe 版本信息里的 FileDescription（本地化产品名），失败返回 None。
fn file_description(exe_path: &str) -> Option<String> {
    unsafe {
        let wide: Vec<u16> = exe_path.encode_utf16().chain(std::iter::once(0)).collect();
        let size = GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        if GetFileVersionInfoW(PCWSTR(wide.as_ptr()), Some(0), size, buf.as_mut_ptr() as *mut c_void)
            .is_err()
        {
            return None;
        }
        // 取第一个语言/代码页块
        let mut tlen: u32 = 0;
        let mut tptr: *mut c_void = std::ptr::null_mut();
        let tkey: Vec<u16> = r"\VarFileInfo\Translation"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        if !VerQueryValueW(
            buf.as_ptr() as *const c_void,
            PCWSTR(tkey.as_ptr()),
            &mut tptr,
            &mut tlen,
        )
        .as_bool()
            || tlen < 4
        {
            return None;
        }
        let lang = *(tptr as *const u16);
        let cp = *((tptr as *const u16).add(1));
        let key = format!(r"\StringFileInfo\{:04x}{:04x}\FileDescription", lang, cp);
        let mut vlen: u32 = 0;
        let mut vptr: *mut c_void = std::ptr::null_mut();
        let vkey: Vec<u16> = key.encode_utf16().chain(std::iter::once(0)).collect();
        if !VerQueryValueW(
            buf.as_ptr() as *const c_void,
            PCWSTR(vkey.as_ptr()),
            &mut vptr,
            &mut vlen,
        )
        .as_bool()
            || vlen == 0
        {
            return None;
        }
        // 注意：VerQueryValueW 的 vlen 是 u16 字符数（含结尾 null），不是字节数。
        // 不能除以 2，否则长名称会被截断一半（如 "Microsoft Edge" → "Microso"）。
        // 以 null 为界扫描最稳妥（兼容 vlen 两种单位语义），杜绝截断与越界。
        let p = vptr as *const u16;
        let max = vlen as usize;
        let mut n = 0usize;
        while n < max && *p.add(n) != 0 {
            n += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
        let s = s.trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
}

// ---------------------------------------------------------------------------
// 图标提取与缓存
// ---------------------------------------------------------------------------

type IconCache = Mutex<HashMap<String, Option<String>>>;

fn icon_cache() -> &'static IconCache {
    static CACHE: OnceLock<IconCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn icon_dir() -> PathBuf {
    crate::config::config_dir().join("icons")
}

fn icon_path(display: &str) -> PathBuf {
    let mut h = DefaultHasher::new();
    display.hash(&mut h);
    icon_dir().join(format!("{:016x}.png", h.finish()))
}

fn png_data_url(png: &[u8]) -> String {
    format!("data:image/png;base64,{}", BASE64.encode(png))
}

/// 前台切换时确保图标已缓存（提取 + 落盘）。同一显示名只提取一次。
/// （pub(crate) 供 audio_usage 复用）
pub(crate) fn ensure_icon(display: &str, exe_path: &str) {
    let mut cache = icon_cache().lock().unwrap();
    if cache.contains_key(display) {
        return;
    }
    let val = match extract_icon_png(exe_path) {
        Some(png) => {
            let _ = std::fs::create_dir_all(icon_dir());
            let _ = std::fs::write(icon_path(display), &png);
            Some(png_data_url(&png))
        }
        None => None,
    };
    cache.insert(display.to_string(), val);
}

/// 查询图标 data URL：内存缓存 → 磁盘缓存。都没有返回 None（前端显示首字占位）。
/// （pub(crate) 供 audio_usage 复用）
pub(crate) fn cached_icon(display: &str) -> Option<String> {
    if let Some(v) = icon_cache().lock().unwrap().get(display) {
        return v.clone();
    }
    if let Ok(png) = std::fs::read(icon_path(display)) {
        let url = png_data_url(&png);
        icon_cache()
            .lock()
            .unwrap()
            .insert(display.to_string(), Some(url.clone()));
        return Some(url);
    }
    None
}

/// 提取 exe 第一个图标 → RGBA → PNG 字节。无图标资源/失败返回 None。
fn extract_icon_png(exe_path: &str) -> Option<Vec<u8>> {
    unsafe {
        let wide: Vec<u16> = exe_path.encode_utf16().chain(std::iter::once(0)).collect();
        let mut hlarge: HICON = HICON::default();
        let mut hsmall: HICON = HICON::default();
        if ExtractIconExW(PCWSTR(wide.as_ptr()), 0, Some(&mut hlarge), Some(&mut hsmall), 1) == 0 {
            return None;
        }
        let icon = if !hlarge.is_invalid() { hlarge } else { hsmall };
        let mut ii = ICONINFO::default();
        if GetIconInfo(icon, &mut ii).is_err() {
            let _ = DestroyIcon(hlarge);
            let _ = DestroyIcon(hsmall);
            return None;
        }
        let mut bmp = BITMAP::default();
        let bmp_ok = !ii.hbmColor.is_invalid()
            && GetObjectW(
                ii.hbmColor.into(),
                std::mem::size_of::<BITMAP>() as i32,
                Some(&mut bmp as *mut _ as *mut c_void),
            ) != 0;
        let _ = DeleteObject(ii.hbmColor.into());
        let _ = DeleteObject(ii.hbmMask.into());
        if !bmp_ok || bmp.bmWidth <= 0 || bmp.bmHeight <= 0 {
            let _ = DestroyIcon(hlarge);
            let _ = DestroyIcon(hsmall);
            return None;
        }
        let w = bmp.bmWidth as u32;
        let h = bmp.bmHeight as u32;
        if w > 128 || h > 128 {
            let _ = DestroyIcon(hlarge);
            let _ = DestroyIcon(hsmall);
            return None;
        }

        let mem_dc = CreateCompatibleDC(None);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let Ok(hbmp) = CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
        else {
            let _ = DeleteDC(mem_dc);
            let _ = DestroyIcon(hlarge);
            let _ = DestroyIcon(hsmall);
            return None;
        };
        if hbmp.is_invalid() || bits.is_null() {
            let _ = DeleteDC(mem_dc);
            let _ = DestroyIcon(hlarge);
            let _ = DestroyIcon(hsmall);
            return None;
        }
        let old = SelectObject(mem_dc, hbmp.into());
        let _ = DrawIconEx(mem_dc, 0, 0, icon, w as i32, h as i32, 0, None, DI_NORMAL);
        // 读 DIB 像素（32bpp top-down = BGRA），转 RGBA
        let len = (w * h * 4) as usize;
        let mut rgba = vec![0u8; len];
        std::ptr::copy_nonoverlapping(bits as *const u8, rgba.as_mut_ptr(), len);
        for px in rgba.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        let _ = SelectObject(mem_dc, old);
        let _ = DeleteObject(hbmp.into());
        let _ = DeleteDC(mem_dc);
        let _ = DestroyIcon(hlarge);
        let _ = DestroyIcon(hsmall);

        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().ok()?;
        writer.write_image_data(&rgba).ok()?;
        drop(writer); // 结束对 out 的借用，之后才能把 out 移出
        Some(out)
    }
}

// ---------------------------------------------------------------------------
// 前台切换识别
// ---------------------------------------------------------------------------

/// 窗口句柄 → (exe 完整路径, 进程名小写)。拿不到（权限/已退出）返回 None。
fn process_info_of(hwnd: HWND) -> Option<(String, String)> {
    let mut pid: u32 = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if pid == 0 {
        return None;
    }
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return None;
        };
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut size);
        let _ = CloseHandle(h);
        if ok.is_ok() && size > 0 {
            let s = String::from_utf16_lossy(&buf[..size as usize]);
            let name = s.rsplit('\\').next().unwrap_or("").to_lowercase();
            return Some((s, name));
        }
    }
    None
}

/// 更新当前前台应用（全量：任何前台窗口都记录，跳过自身进程）
fn update_cur_app(hwnd: HWND) {
    let Some((exe_path, exe_name)) = process_info_of(hwnd) else {
        *CUR_APP.lock().unwrap() = None;
        return;
    };
    if exe_name == SELF_EXE {
        *CUR_APP.lock().unwrap() = None;
        return;
    }
    let display = display_name_of(&exe_path, &exe_name);
    ensure_icon(&display, &exe_path);
    *CUR_APP.lock().unwrap() = Some(CurApp { display });
}

// ---------------------------------------------------------------------------
// 事件钩子
// ---------------------------------------------------------------------------

unsafe extern "system" fn win_event_proc(
    _hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    _id_obj: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if event == EVENT_SYSTEM_FOREGROUND && ENABLED.load(Ordering::Relaxed) {
        update_cur_app(hwnd);
    }
}

unsafe fn watch_thread() {
    let h = SetWinEventHook(
        EVENT_SYSTEM_FOREGROUND,
        EVENT_SYSTEM_FOREGROUND,
        None,
        Some(win_event_proc),
        0,
        0,
        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
    );
    if h.is_invalid() {
        eprintln!(
            "[app_usage] 前台窗口事件钩子安装失败: {}",
            windows::core::Error::from_win32()
        );
        WATCH_OK.store(false, Ordering::SeqCst);
        return;
    }
    WATCH_OK.store(true, Ordering::SeqCst);
    // 启动时初始化：若此刻正有前台窗口，立即进入统计
    let fg = GetForegroundWindow();
    if !fg.is_invalid() {
        update_cur_app(fg);
    }
    crate::win::run_message_loop();
    let _ = UnhookWinEvent(h);
}

// ---------------------------------------------------------------------------
// 结算与查询
// ---------------------------------------------------------------------------

/// 每 10 秒结算：若前台有应用且未挂机，按真实时间差累加秒数。
fn tick() {
    let now = now_ms();
    let last = LAST_TICK_MS.swap(now, Ordering::Relaxed);
    if last == 0 {
        return; // 首个 tick 只建立基准
    }
    let delta = (now - last).min(60_000) / 1000;
    if delta < 1 {
        return;
    }
    // 关闭期间不累计（基准已在上面刷新，重开时从当前时刻起算，无跳变）
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let Some(cur) = CUR_APP.lock().unwrap().clone() else {
        return;
    };
    // 挂机判定：距最后一次输入超过阈值 → 整段不算
    let idle = now.saturating_sub(crate::activity::last_input_ms());
    if idle > IDLE_THRESHOLD_MS {
        return;
    }
    let now_dt = Local::now();
    let date = now_dt.date_naive().format("%Y-%m-%d").to_string();
    let hour = now_dt.hour().min(23) as i64;
    let g = crate::db::conn().lock().unwrap();
    let _ = g.execute(
        "INSERT INTO app_usage (date, app, seconds) VALUES (?1, ?2, ?3) \
         ON CONFLICT(date, app) DO UPDATE SET seconds = seconds + ?3",
        params![date, cur.display, delta as i64],
    );
    let _ = g.execute(
        "INSERT INTO app_usage_hourly (date, hour, app, seconds) VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(date, hour, app) DO UPDATE SET seconds = seconds + ?4",
        params![date, hour, cur.display, delta as i64],
    );
}

/// 启动监控：前台事件钩子线程 + 10 秒结算线程。幂等，仅首次生效。
pub fn start() {
    if WATCH_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| unsafe { watch_thread() });
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(10));
        tick();
    });
}

/// 程序退出前：最后结算一把（正常退出零丢失）
pub fn shutdown() {
    tick();
}

// ---------------------------------------------------------------------------
// 查询（get_app_usage_summary 命令）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct AppUsageItem {
    pub app: String,
    pub seconds: i64,
    /// 应用图标（base64 PNG data URL），无图标为 None（前端显示首字占位）
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppUsageSummary {
    /// 统计日期 "YYYY-MM-DD"
    pub date: String,
    /// 今日各应用累计时长（按时长倒序）
    pub apps: Vec<AppUsageItem>,
    /// 24 个小时桶：今日所有应用在该小时的合计秒数（未命中小时为 0）
    pub hourly: Vec<i64>,
    /// 前台监控是否生效
    pub watch_ok: bool,
}

/// 今日应用使用汇总
pub fn summary() -> AppUsageSummary {
    let date = Local::now().date_naive().format("%Y-%m-%d").to_string();
    let g = crate::db::conn().lock().unwrap();

    let mut apps = Vec::new();
    match g.prepare(
        "SELECT app, seconds FROM app_usage WHERE date = ?1 ORDER BY seconds DESC",
    ) {
        Ok(mut stmt) => match stmt.query_map(params![date], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        }) {
            Ok(rows) => {
                for row in rows.flatten() {
                    let app = row.0;
                    let icon = cached_icon(&app);
                    apps.push(AppUsageItem {
                        app,
                        seconds: row.1,
                        icon,
                    });
                }
            }
            Err(_) => {}
        },
        Err(_) => {}
    }

    let mut hourly = vec![0i64; 24];
    match g.prepare(
        "SELECT hour, SUM(seconds) FROM app_usage_hourly \
         WHERE date = ?1 GROUP BY hour ORDER BY hour",
    ) {
        Ok(mut stmt) => match stmt.query_map(params![date], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
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

    AppUsageSummary {
        date,
        apps,
        hourly,
        watch_ok: WATCH_OK.load(Ordering::SeqCst),
    }
}
