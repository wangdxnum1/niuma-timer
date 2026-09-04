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
//! - 结算：**切换驱动，而非 tick 驱动**。前台一切换先把上一段时长结给「切走前」的
//!   应用，常驻线程的 10 秒 tick 只负责切段兜底。若只在 tick 时按时间差累加，
//!   两次 tick 之间切走的那一整段会被错记给切换后的应用（微信看 8 秒再切走 →
//!   Chrome 白得 10 秒）。最终写入 SQLite 两张表（app_usage 天级汇总 +
//!   app_usage_hourly 小时分布）。
//!
//! 统计生效范围：程序运行期间（App 常驻托盘即持续统计），跳过自身进程。

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::ffi::c_void;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use chrono::{Local, TimeZone, Timelike};
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
use windows::Win32::UI::Shell::ExtractIconExW;
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, DrawIconEx, GetIconInfo, GetWindowThreadProcessId, DI_NORMAL, EVENT_SYSTEM_FOREGROUND,
    HICON, ICONINFO,
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

/// 自身进程名：前台窗口是自己时不统计
pub(crate) const SELF_EXE: &str = "niuma-timer.exe";

/// 当前前台命中的应用
#[derive(Clone)]
struct CurApp {
    /// 显示名（软件名，如「微信」「钉钉」）
    display: String,
    /// exe 完整路径（用于懒提取图标，避免在前台切换回调热路径里同步做 GDI+PNG）
    exe_path: String,
}

/// 当前前台应用 + 「本段」起始时刻。
///
/// 关键不变量：**时长归属由前台切换驱动，而不是只靠 10 秒 tick**。
/// 旧实现只在 tick 时把整个 delta 记给「此刻」的前台应用，于是两次 tick 之间
/// 切换应用会把整段时间错记给切换后的那个（微信看 8 秒再切走 → Chrome 白得 10 秒）。
/// 现在前台一切换就先把上一段结给「切走前」的应用，tick 只负责切段兜底。
struct CurState {
    app: Option<CurApp>,
    /// 本段起始时刻（**单调**毫秒，取自 `mono_ms`，不是墙钟）。
    /// 0 = 尚未建立基准（首次只建基准、不计秒）
    since_ms: u64,
}

static CUR: Mutex<CurState> = Mutex::new(CurState {
    app: None,
    since_ms: 0,
});

/// 「显示名 → exe 路径」映射：前台切换时填充，供 summary() 查询路径懒提取图标，
/// 覆盖「仅短暂前台、tick 来不及提取」的边界情况，避免明细里出现首字占位图标。
/// （HashMap::new 不是 const fn，不能用在 static 直接量里，故用 OnceLock 延迟初始化）
fn app_exe_map() -> &'static Mutex<HashMap<String, String>> {
    static MAP: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 单段最长计入时长（毫秒）：系统睡眠/休眠醒来后 now 会大幅跳变，
/// 此时最多补记 60 秒，避免把整段睡眠时长记成应用使用时长。
const MAX_SEGMENT_MS: u64 = 60_000;

static WATCH_STARTED: AtomicBool = AtomicBool::new(false);
static WATCH_OK: AtomicBool = AtomicBool::new(false);

/// 应用使用监控总开关（设置页可切换）。关闭时前台切换事件被忽略、不累计时长。
pub static ENABLED: AtomicBool = AtomicBool::new(true);

/// 运行时切换应用使用监控（立即生效）
pub fn set_enabled(v: bool) {
    ENABLED.store(v, Ordering::SeqCst);
}

/// 应用使用白名单（设置页可维护）：开启且名单非空时，仅名单内应用计入使用统计。
pub static WHITELIST_ENABLED: AtomicBool = AtomicBool::new(false);
static WHITELIST: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// 运行时切换应用白名单（启动与保存配置时调用，立即生效）
pub fn set_whitelist(enabled: bool, list: Vec<String>) {
    WHITELIST_ENABLED.store(enabled, Ordering::SeqCst);
    *WHITELIST.lock().unwrap() = list;
}

/// 当前应用是否纳入统计：未开启白名单或名单为空 → 统计全部；否则按展示名（大小写不敏感）匹配。
fn in_whitelist(display: &str) -> bool {
    if !WHITELIST_ENABLED.load(Ordering::Relaxed) {
        return true;
    }
    let list = WHITELIST.lock().unwrap();
    if list.is_empty() {
        return true;
    }
    let d = display.to_lowercase();
    list.iter().any(|w| w.to_lowercase() == d)
}

/// 重新开启监控时调用：把当前前台窗口立即纳入统计，无需等下次窗口切换
pub fn refresh_foreground() {
    if let Some(fg) = crate::win::foreground_window() {
        switch_foreground(fg);
    }
}

/// 墙钟毫秒（自 Unix 纪元）。
///
/// 只用于需要与「墙钟时刻」比较的场合：目前是挂机判定——
/// `activity::last_input_ms()` 返回的也是墙钟毫秒，两者必须同域才能相减。
///
/// 本项目三种时间各司其职，不可混用：
/// - 段时长 → 单调时钟 `mono_ms()`（免疫回拨，只关心间距）；
/// - 挂机判定 → 墙钟 `now_ms()`（与 `last_input_ms()` 同域）；
/// - 落库的 `date` / `hour` → `chrono::Local::now()`（要的是日历日期）。
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 单调毫秒：进程启动后经过的毫秒数（带 `+1` 偏移）。
///
/// 基于 `Instant`，**不受系统时间回拨 / NTP 校正 / 手动改时间影响**，
/// 专供计算时间差（段时长这类「只关心间距」的量）。
///
/// 为什么段时长必须用单调时钟：墙钟一旦回拨，`now < since` 会让 `saturating_sub`
/// 得 0，段被丢弃的同时段起点也不推进——此后在墙钟追上 `since` 之前，
/// 应用使用时长会**永久停记**。单调时钟不会回退，从根上消除这类滞留。
///
/// `+1` 是为了避开 `CurState.since_ms == 0` 这个「尚未建立基准」的哨兵值：
/// `Instant` 起算时可能正好返回 0，撞上哨兵会被误判成「首次调用」而白丢第一段。
fn mono_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
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
        CUR.lock().unwrap().app = None;
        return;
    };
    if exe_name == SELF_EXE {
        CUR.lock().unwrap().app = None;
        return;
    }
    let display = display_name_of(&exe_path, &exe_name);
    // 白名单过滤：开启且不在名单内 → 不纳入统计（app 置空，结算时不会累加该时段）
    if !in_whitelist(&display) {
        CUR.lock().unwrap().app = None;
        return;
    }
    // 注意：图标不再在这里（前台切换回调，热路径）同步提取，改为在 tick() 后台线程懒提取，
    // 避免 GDI 提取 + PNG 编码 + 落盘阻塞切换识别造成卡顿。
    // 记录「显示名→exe路径」，供 summary() 懒提取覆盖 tick 未来得及提取的边界情况。
    app_exe_map()
        .lock()
        .unwrap()
        .insert(display.clone(), exe_path.clone());
    CUR.lock().unwrap().app = Some(CurApp { display, exe_path });
}

/// 前台应用切换的统一入口：**先把上一段时长结给「切走前」的应用，再记录新的前台应用**。
///
/// 顺序不能反——反了就会把上一段时长记到新应用头上，正是本修复要消除的问题。
/// 图标提取在此关掉（前台切换回调是热路径），由 10 秒 tick 后台补齐。
fn switch_foreground(hwnd: HWND) {
    settle(mono_ms(), false);
    update_cur_app(hwnd);
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
        switch_foreground(hwnd);
    }
}

fn watch_thread() {
    // RAII 守卫持有钩子：本线程无论正常退出还是 panic，Drop 都会 UnhookWinEvent，
    // 不会再出现「提前 return 漏卸载」的手工配对问题（收口于 win::WinEventHookGuard）。
    let Some(_hook) = crate::win::WinEventHookGuard::install_foreground_hook(Some(win_event_proc))
    else {
        eprintln!(
            "[app_usage] 前台窗口事件钩子安装失败: {}",
            windows::core::Error::from_win32()
        );
        WATCH_OK.store(false, Ordering::SeqCst);
        return;
    };
    WATCH_OK.store(true, Ordering::SeqCst);
    // 启动时初始化：若此刻正有前台窗口，立即进入统计（此刻段起点为 0，只建基准不计秒）
    if let Some(fg) = crate::win::foreground_window() {
        switch_foreground(fg);
    }
    // SAFETY：OUTOFCONTEXT 钩子的回调投递到本线程，必须跑消息循环才能收到事件
    // （见 win.rs 顶部不变量）。
    unsafe { crate::win::run_message_loop() };
}

// ---------------------------------------------------------------------------
// 结算与查询
// ---------------------------------------------------------------------------

/// 取出「上一段」并推进段起点：返回该段归属的前台应用与应计入的整秒数。
///
/// `now` 是**单调**毫秒（取自 `mono_ms`），不是墙钟：段时长只关心间距，用单调时钟
/// 可免疫系统时间回拨 / NTP 校正 / 手动改时间。用墙钟时，一旦回拨导致 `now < since`，
/// `saturating_sub` 得 0，段被丢弃的同时段起点也不推进——在墙钟追上 `since` 之前，
/// 应用使用时长会**永久停记**。单调时钟不回退，从根上消除这种滞留。
///
/// 段起点只推进 `secs * 1000`（而不是直接跳到 now）：不足 1 秒的零头留到下一段，
/// 否则高频切换时每段零头都被抹掉（切 1000 次 × 0.9 秒 → 一天凭空丢 15 分钟）。
/// 只有长时间未结算（睡眠 / 休眠）时才把起点直接推到 now，把超限部分丢弃。
fn take_segment(now: u64) -> (Option<CurApp>, u64) {
    let mut g = CUR.lock().unwrap();
    let since = g.since_ms;
    if since == 0 {
        g.since_ms = now; // 首次只建立基准
        return (None, 0);
    }
    let raw = now.saturating_sub(since);
    if raw > MAX_SEGMENT_MS {
        g.since_ms = now; // 时间跳变：最多补记一段，其余丢弃
        return (g.app.clone(), MAX_SEGMENT_MS / 1000);
    }
    let secs = raw / 1000;
    g.since_ms = since + secs * 1000;
    (g.app.clone(), secs)
}

/// 结算上一段：把「自上次结算以来」的真实时长记给该段的前台应用。
///
/// 由两处调用：前台切换回调（切走前先结账）与 10 秒 tick（切段兜底）。
///
/// `with_icon` 为 false 时跳过图标提取——前台切换回调是热路径，绝不能在那里做
/// GDI 提取 + PNG 编码 + 落盘（毫秒级）；图标统一由 10 秒 tick 在后台补齐。
///
/// 留在回调里做的是 SQLite 写入，量级完全不同：WAL + synchronous=NORMAL 下一次
/// upsert 几十微秒、不做 fsync，而 Windows 对事件回调的容忍在数百毫秒级，
/// 即便疯狂切窗口也不会触发系统弃用回调，故不必为此再引入一层队列。
fn settle(now_mono: u64, with_icon: bool) {
    // 段已取出、段起点已推进：下面任何一道门控命中都只是「丢弃这一段」，
    // 不会造成基准滞留（解锁 / 重开监控后从当前时刻起算，无跳变）。
    let (app, secs) = take_segment(now_mono);
    if secs == 0 {
        return;
    }
    let Some(cur) = app else {
        return; // 无前台 / 自身进程 / 不在白名单内
    };
    // 锁屏离开期间不统计应用使用时间
    if crate::lock_monitor::is_away() {
        return;
    }
    // 关闭期间不累计
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    // 挂机判定：距最后一次输入超过阈值 → 整段不算。
    // 这里必须用墙钟 `now_ms()`：`activity::last_input_ms()` 返回的也是墙钟毫秒，
    // 只有同域相减才有意义（段时长已改用单调时钟，两者不可混用）。
    let idle = now_ms().saturating_sub(crate::activity::last_input_ms());
    if idle > IDLE_THRESHOLD_MS {
        return;
    }
    if with_icon {
        // 懒提取当前应用图标（后台线程，非热路径）：首次命中时 GDI+PNG 一次，之后走缓存
        ensure_icon(&cur.display, &cur.exe_path);
    }
    // 本段覆盖的墙钟区间：时长 `secs` 取自单调时钟（准确），结束时刻取结算瞬间的墙钟。
    // 段长恰为 secs 秒（不足 1 秒的零头留给下一段），故按 [end - secs*1000, end) 还原。
    // 注意：零头会让整段位置有 <1 秒的偏移，仅影响边界处 1 秒的归属，可接受——
    // 相比旧行为「整段（最长 60 秒）被吞并到边界后的桶」，误差小了两个数量级。
    let end_wall = now_ms();
    let start_wall = end_wall.saturating_sub(secs.saturating_mul(1000));
    // 按整点 / 跨天边界切分后分桶记账，避免整段被吞并到结算时刻所在的那个桶（A5）。
    let parts = split_by_hour(start_wall, end_wall, secs);
    let g = crate::db::conn().lock().unwrap();
    for (date, hour, s) in parts {
        let _ = g.execute(
            "INSERT INTO app_usage (date, app, seconds) VALUES (?1, ?2, ?3) \
             ON CONFLICT(date, app) DO UPDATE SET seconds = seconds + ?3",
            params![date, cur.display, s],
        );
        let _ = g.execute(
            "INSERT INTO app_usage_hourly (date, hour, app, seconds) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(date, hour, app) DO UPDATE SET seconds = seconds + ?4",
            params![date, hour, cur.display, s],
        );
    }
}

/// 把一段墙钟区间按「整点（含跨天）」边界切分，供落库时分桶记账。
///
/// 背景（A5）：原实现按**结算时刻**所在的 date/hour 把整段 `secs` 全记进去，
/// 于是 23:59:55 → 00:00:05 这 10 秒会整段落到「新一天的 0 点」桶，
/// 前一个小时 / 前一天被白白吞掉。这里先按边界把段切开，各归各桶。
///
/// 返回值是 `(date, hour, seconds)` 列表，**总和恒等于传入的 `secs`**：
/// 切分产生的不足 1 秒零头会补给最后一个子段，既不丢秒也不重复记。
///
/// 注意这里必须写 `chrono::Duration` 全路径——本文件已导入 `std::time::Duration`，
/// 两者同名，直接写 `Duration` 会拿到 std 的版本而编译失败。
fn split_by_hour(start_ms: u64, end_ms: u64, secs: u64) -> Vec<(String, i64, i64)> {
    let mut out: Vec<(String, i64, i64)> = Vec::new();
    if secs == 0 || end_ms <= start_ms {
        return out;
    }
    let mut remaining = secs;
    let mut cur = start_ms;
    while cur < end_ms && remaining > 0 {
        let dt = match Local.timestamp_millis_opt(cur as i64).single() {
            Some(d) => d,
            None => break, // 时间戳越界：放弃切分，走下面的保底分支
        };
        // 本子段所属小时桶的结束时刻（下一个整点）；chrono 自动处理跨天
        let next_ms = (dt + chrono::Duration::hours(1))
            .with_minute(0)
            .and_then(|d| d.with_second(0))
            .and_then(|d| d.with_nanosecond(0))
            .map(|d| d.timestamp_millis().max(0) as u64)
            .unwrap_or(end_ms);
        let seg_end = next_ms.min(end_ms);
        if seg_end <= cur {
            break; // 防御：时钟异常时不要死循环
        }
        let sub_secs = (seg_end.saturating_sub(cur) / 1000).min(remaining);
        if sub_secs > 0 {
            out.push((
                dt.date_naive().format("%Y-%m-%d").to_string(),
                dt.hour().min(23) as i64,
                sub_secs as i64,
            ));
            remaining -= sub_secs;
        }
        cur = seg_end;
    }
    // 各子段按整秒取整后若有残留（每段不足 1 秒的零头），补给最后一个子段；
    // 若一个子段都没切出来（整段不足 1 秒 / 时间戳越界），整段记到起点所属桶。
    if remaining > 0 {
        if let Some(last) = out.last_mut() {
            last.2 += remaining as i64;
        } else {
            let dt = Local
                .timestamp_millis_opt(start_ms as i64)
                .single()
                .unwrap_or_else(Local::now);
            out.push((
                dt.date_naive().format("%Y-%m-%d").to_string(),
                dt.hour().min(23) as i64,
                secs as i64,
            ));
        }
    }
    out
}

/// 每 10 秒结算一次：把自上次结算以来的时长结给当前前台应用。
/// 正常无切换时等价旧行为；有切换时由切换回调先结过账，这里只切段兜底。
fn tick() {
    settle(mono_ms(), true);
}

/// 启动监控：前台事件钩子线程 + 10 秒结算线程。幂等，仅首次生效。
pub fn start() {
    if WATCH_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(watch_thread);
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
                    // 懒提取：若尚未缓存图标，用记录的 exe 路径补提取（GDI+PNG 一次，幂等）
                    if let Some(exe) = app_exe_map().lock().unwrap().get(&app).cloned() {
                        ensure_icon(&app, &exe);
                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `CUR` 是全局状态，而 cargo test 默认多线程并发跑用例，故访问它的用例先抢这把锁。
    /// 容忍中毒：单个用例断言失败不应把其余用例连坐成 panic。
    static TEST_LOCK: Mutex<()> = Mutex::new(());
    fn lock_cur() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 单调时钟必须真前进，且永不返回 0。
    ///
    /// 这两条正是 A4 的核心不变量：
    /// ① 段时长改用 `Instant` 后不回退，系统时间回拨 / NTP 校正不会让段起点滞留；
    /// ② 0 是 `since_ms` 的「尚未建立基准」哨兵，`mono_ms()` 一旦返回 0，
    ///    首次建立基准就会被误判成「还没建过」，白丢第一段。
    #[test]
    fn mono_ms_is_monotonic_and_nonzero() {
        let a = mono_ms();
        assert!(a > 0, "mono_ms 不得返回 0，会撞上 since_ms 的未初始化哨兵");
        std::thread::sleep(Duration::from_millis(20));
        let b = mono_ms();
        assert!(b > a, "单调时钟必须随时间前进: {a} -> {b}");
        assert!(b - a >= 15, "睡了 20ms，差值不应明显偏小: {}", b - a);
    }

    /// 段切分的三条关键性质。
    ///
    /// 必须写在**同一个**测试函数里顺序执行：三步共用同一个段起点，拆开会互相覆盖。
    ///
    /// 第三条尤其重要：时间跳变（睡眠/休眠）封顶后「必须把段起点推到 now」——
    /// 若只按整秒推进，多出来的 3540 秒会变成补记债，接下来几十次结算每次再送 60 秒。
    #[test]
    fn take_segment_advances_baseline_correctly() {
        let _g = lock_cur();
        // ① 首次调用只建立基准，不计秒
        {
            let mut g = CUR.lock().unwrap();
            g.app = None;
            g.since_ms = 0;
        }
        let (_, secs) = take_segment(1_000_000);
        assert_eq!(secs, 0, "首次调用只建基准，不应计秒");
        assert_eq!(CUR.lock().unwrap().since_ms, 1_000_000);

        // ② 正常段：返回整秒数，起点只按整秒推进 → 不足 1 秒的零头留到下一段
        let (_, secs) = take_segment(1_010_900);
        assert_eq!(secs, 10, "10.9 秒应只计 10 秒");
        assert_eq!(
            CUR.lock().unwrap().since_ms,
            1_010_000,
            "零头 900ms 必须留给下一段，而不是被抹掉"
        );
        // 紧接着再切一次，零头与上一段的 900ms 合并成 1 秒
        let (_, secs) = take_segment(1_011_000);
        assert_eq!(secs, 1, "上一段零头 900ms + 本段 100ms 应合计 1 秒");
        assert_eq!(CUR.lock().unwrap().since_ms, 1_011_000);

        // ③ 时间跳变：封顶 60 秒，且起点直接推到 now（不留补记债）
        let (_, secs) = take_segment(1_011_000 + 3_600_000);
        assert_eq!(secs, 60, "睡眠 1 小时最多只补记 60 秒");
        assert_eq!(
            CUR.lock().unwrap().since_ms,
            1_011_000 + 3_600_000,
            "封顶后起点必须推到 now，否则残留时间会被反复补记"
        );
        // 跳变之后回到正常切段：不得把残留的 3540 秒分批补记出来
        let (_, secs) = take_segment(1_011_000 + 3_600_000 + 10_000);
        assert_eq!(secs, 10, "跳变之后应回到正常切段，而不是继续补记");
    }

    /// 切走前先结账：切换后上一应用的时长不得记到新应用头上。
    /// 只验证「段归属取的是切换前的 app 快照」这一契约，不碰数据库。
    #[test]
    fn segment_is_attributed_to_outgoing_app() {
        let _g = lock_cur();
        let old = CurApp {
            display: "微信".into(),
            exe_path: "C:/WeChat.exe".into(),
        };
        {
            let mut g = CUR.lock().unwrap();
            g.app = Some(old.clone());
            g.since_ms = 2_000_000;
        }
        // 模拟「微信用了 9 秒后切走」：先取段（此时 app 仍是微信）
        let (app, secs) = take_segment(2_009_000);
        assert_eq!(secs, 9);
        assert_eq!(
            app.map(|a| a.display),
            Some("微信".into()),
            "9 秒必须记给切走前的微信"
        );
        // 再更新为新应用
        CUR.lock().unwrap().app = Some(CurApp {
            display: "Chrome".into(),
            exe_path: "C:/chrome.exe".into(),
        });
        // 之后的时长才归 Chrome
        let (app, secs) = take_segment(2_010_000);
        assert_eq!(secs, 1);
        assert_eq!(app.map(|a| a.display), Some("Chrome".into()));
    }

    /// 跨小时 / 跨天必须按边界分桶，且切分前后总秒数守恒（A5）。
    ///
    /// 不碰 `CUR`，无需抢 TEST_LOCK。
    #[test]
    fn split_by_hour_splits_across_hour_and_day() {
        // ① 同一小时内：不切分，整段归该小时
        let start = Local
            .with_ymd_and_hms(2026, 9, 3, 10, 0, 0)
            .unwrap()
            .timestamp_millis() as u64;
        let parts = split_by_hour(start, start + 10_000, 10);
        assert_eq!(parts.len(), 1, "同一小时内不该切分");
        assert_eq!(parts[0].1, 10);
        assert_eq!(parts[0].2, 10);

        // ② 跨小时不跨天：10:59:55 起的 10 秒 → 10 点 5 秒 + 11 点 5 秒
        let start = Local
            .with_ymd_and_hms(2026, 9, 3, 10, 59, 55)
            .unwrap()
            .timestamp_millis() as u64;
        let parts = split_by_hour(start, start + 10_000, 10);
        assert_eq!(parts.len(), 2, "跨整点必须切成两段");
        assert_eq!(parts[0].1, 10);
        assert_eq!(parts[0].2, 5);
        assert_eq!(parts[1].1, 11);
        assert_eq!(parts[1].2, 5);

        // ③ 跨天：23:59:55 起的 10 秒 → 当天 23 点 5 秒 + 次日 0 点 5 秒，日期必须不同
        let start = Local
            .with_ymd_and_hms(2026, 9, 3, 23, 59, 55)
            .unwrap()
            .timestamp_millis() as u64;
        let parts = split_by_hour(start, start + 10_000, 10);
        assert_eq!(parts.len(), 2, "跨天必须切成两段");
        assert_eq!(parts[0].1, 23);
        assert_eq!(parts[0].2, 5);
        assert_eq!(parts[1].1, 0);
        assert_eq!(parts[1].2, 5);
        assert_ne!(parts[0].0, parts[1].0, "跨天后两段应落在不同日期");

        // ④ 无论怎么切，总秒数必须守恒（不丢秒也不重复记）
        let cases = [
            (
                Local
                    .with_ymd_and_hms(2026, 9, 3, 10, 59, 55)
                    .unwrap()
                    .timestamp_millis() as u64,
                10u64,
            ),
            (
                Local
                    .with_ymd_and_hms(2026, 9, 3, 23, 59, 59)
                    .unwrap()
                    .timestamp_millis() as u64,
                60u64,
            ),
            (
                Local
                    .with_ymd_and_hms(2026, 9, 3, 8, 0, 0)
                    .unwrap()
                    .timestamp_millis() as u64,
                1u64,
            ),
        ];
        for (start, secs) in cases {
            let parts = split_by_hour(start, start + secs * 1000, secs);
            let sum: i64 = parts.iter().map(|p| p.2).sum();
            assert_eq!(sum, secs as i64, "切分后总秒数必须守恒: {parts:?}");
        }
    }
}
