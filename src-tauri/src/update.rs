//! 自动更新（v1.4.0 阶段 A）：版本发现、版本比较、更新状态机。
//! 本文件先只放「纯逻辑」——可单测、无副作用；网络 / 注册表 / 文件替换等 IO
//! 由后续任务（A5 助手、A7 命令）以窄接口注入，便于用假实现驱动测试。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::Manager;
use std::cmp::Ordering;
use std::collections::HashMap;

static UPDATE_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub struct UpdateOperation;
impl UpdateOperation {
    pub fn begin() -> Result<Self, String> {
        UPDATE_BUSY.compare_exchange(false, true, std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst).map(|_| Self).map_err(|_| "更新正在进行，请勿重复操作".into())
    }
}
impl Drop for UpdateOperation {
    fn drop(&mut self) { UPDATE_BUSY.store(false, std::sync::atomic::Ordering::SeqCst); }
}

fn helper_lock(target: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .share_mode(0).open(target.with_extension("update.lock"))
}

// The file may remain after a crash; ownership is the OS handle, not its existence.
fn wait_for_helper(target: &Path) -> Result<bool, String> {
    let mut waited = false;
    for _ in 0..1200 {
        match helper_lock(target) {
            Ok(_lock) => return Ok(waited),
            Err(e) if e.raw_os_error() == Some(32) => {
                waited = true;
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) if !waited => return Ok(false), // installed/read-only directory
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("更新仍在进行，请稍后启动".into())
}

pub fn resume_after_helper() -> Result<bool, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if wait_for_helper(&exe)? {
        // This process may have mapped the OLD image before replacement. Never initialize it.
        std::process::Command::new(&exe).spawn().map_err(|e| e.to_string())?;
        return Ok(true);
    }
    Ok(false)
}

#[cfg(test)]
mod concurrency_tests {
    use super::*;
    #[test]
    fn update_operation_is_exclusive_and_released() {
        let first = UpdateOperation::begin().unwrap();
        assert!(UpdateOperation::begin().is_err());
        drop(first);
        assert!(UpdateOperation::begin().is_ok());
    }
    #[test]
    fn helper_lock_excludes_second_helper_and_startup_waits() {
        let target = std::env::temp_dir().join(format!("niuma-lock-test-{}.exe", std::process::id()));
        let lock = helper_lock(&target).unwrap();
        assert!(helper_lock(&target).is_err());
        let path = target.clone();
        let waiter = std::thread::spawn(move || wait_for_helper(&path));
        std::thread::sleep(Duration::from_millis(150));
        drop(lock);
        assert!(waiter.join().unwrap().unwrap());
        assert!(!wait_for_helper(&target).unwrap());
        let _ = std::fs::remove_file(target.with_extension("update.lock"));
    }
}

/// 远端 latest.json 中单个平台的条目
#[derive(Debug, Clone, Deserialize)]
pub struct PlatformEntry {
    /// minisign 签名内容（安装版由 updater 强制校验）
    #[serde(default)]
    pub signature: String,
    /// 安装包 / 绿色版下载地址
    pub url: String,
}

/// 远端 latest.json（Tauri updater 约定结构，仅保留本项目用到的字段）
#[derive(Debug, Clone, Deserialize)]
pub struct RemoteRelease {
    /// 远端版本号
    pub version: String,
    /// 更新说明（复用 CHANGELOG 段落）
    #[serde(default)]
    pub notes: Option<String>,
    /// 发布时间（RFC3339）
    #[serde(default)]
    pub pub_date: Option<String>,
    /// 平台键 → 条目
    #[serde(default)]
    pub platforms: HashMap<String, PlatformEntry>,
}

impl RemoteRelease {
    /// 取指定平台键的下载地址（绿色版用 portable 键；安装版用 windows-x86_64 键）
    pub fn platform_url(&self, key: &str) -> Option<&str> {
        self.platforms.get(key).map(|p| p.url.as_str())
    }
}

/// `--apply-update` 助手模式参数
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperArgs {
    /// 当前 exe 绝对路径（被替换的目标）
    pub target: String,
    /// 需等待其退出的父进程 pid
    pub wait_pid: Option<u32>,
    /// 新 exe 下载地址
    pub url: String,
    /// 期望的 SHA256（小写十六进制）
    pub sha256: String,
}

/// 手写语义化版本比较（不引入 semver 依赖）：
/// 数字段逐段数值比较（`1.10.0 > 1.9.0`），不等长按缺位补 0（`1.4 == 1.4.0`），
/// 主版本相同时「正式版 > 预发布版」（`1.4.0 > 1.4.0-beta.1`）。
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    // 拆成「数字段 Vec<u64>」+「预发布后缀 Option<String>」
    fn split(s: &str) -> (Vec<u64>, Option<String>) {
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, Some(p.to_string())),
            None => (s, None),
        };
        let nums = core
            .split('.')
            .map(|x| x.trim().parse::<u64>().unwrap_or(0))
            .collect::<Vec<u64>>();
        (nums, pre)
    }

    let (mut na, pre_a) = split(a);
    let (mut nb, pre_b) = split(b);
    // 不等长按缺位补 0：1.4 与 1.4.0 视为相等
    let len = na.len().max(nb.len());
    na.resize(len, 0);
    nb.resize(len, 0);
    for i in 0..len {
        let ord = na[i].cmp(&nb[i]);
        if ord != Ordering::Equal {
            return ord;
        }
    }
    // 主版本相同：正式版 > 预发布版；两者都有后缀时按字符串比（beta.1 < beta.2）
    match (pre_a, pre_b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => x.cmp(&y),
    }
}

/// 解析远端 latest.json；缺 `version` / 非 JSON / 结构不符均返回明确 Err。
pub fn parse_latest_json(s: &str) -> Result<RemoteRelease, String> {
    let rel: RemoteRelease =
        serde_json::from_str(s).map_err(|e| format!("latest.json 解析失败: {e}"))?;
    if rel.version.trim().is_empty() {
        return Err("latest.json 缺少 version 字段".to_string());
    }
    Ok(rel)
}

/// 综合「远端是否有更新」与「用户是否跳过该版本」判断是否应提示。
pub fn should_notify(remote: &str, current: &str, skipped: Option<&str>) -> bool {
    // 仅在远端确实更新时提示；用户已跳过该确切版本则静默
    compare_versions(remote, current) == Ordering::Greater && skipped != Some(remote)
}

/// 从进程 argv 解析 `--apply-update` 助手参数；未命中该开关时返回 None。
pub fn parse_helper_args(argv: &[String]) -> Option<HelperArgs> {
    if !argv.iter().any(|a| a == "--apply-update") {
        return None;
    }
    let value_of = |key: &str| -> Option<String> {
        argv.iter()
            .position(|a| a == key)
            .and_then(|i| argv.get(i + 1))
            .cloned()
    };
    Some(HelperArgs {
        target: value_of("--target")?,
        wait_pid: value_of("--wait-pid").and_then(|s| s.parse::<u32>().ok()),
        url: value_of("--url")?,
        sha256: value_of("--sha256")?.to_lowercase(),
    })
}

/// 助手流程的最大等待轮询次数（60 × 500ms ≈ 30s）。
///
/// 用「轮询计数」而非墙钟 deadline：单测里假 IO 立即返回、间隔为 0，
/// 超时分支能秒级跑完，不必真的 sleep 30 秒。
const WAIT_MAX_POLLS: u32 = 60;

/// 助手流程所需的最小 IO 能力。
///
/// 状态机 `run_helper_flow` 只依赖本 trait，真实实现走系统调用，
/// 单测用假实现注入「下载内容 / 重命名失败 / 进程僵死」，无需碰真实文件与网络。
pub trait HelperIo {
    /// 指定 pid 的进程是否仍存活
    fn is_running(&self, pid: u32) -> bool;
    /// 把 url 的内容下载到内存
    fn download(&self, url: &str) -> Result<Vec<u8>, String>;
    /// 写出字节到文件（覆盖）
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), String>;
    /// 重命名 / 覆盖移动
    fn rename(&self, from: &Path, to: &Path) -> Result<(), String>;
    /// 删除文件（文件不存在视为成功）
    fn remove(&self, path: &Path) -> Result<(), String>;
    /// 拉起新 exe（不等待其退出）
    fn spawn(&self, path: &Path) -> Result<(), String>;
    /// 记一行助手日志（发布版无控制台，绝不用 println!）
    fn log(&self, msg: &str);
    /// 轮询间隔（毫秒）；单测返回 0
    fn poll_interval_ms(&self) -> u64;
}

/// 助手状态机：等旧进程退出 → 下载 → 校验 SHA256 → 写 `.new` → 备份 `.old`
/// → 顶替（失败回滚）→ 拉新版 → 清理备份。
///
/// 返回进程退出码：0 成功，1 失败（失败原因全部进日志）。
pub fn run_helper_flow<IO: HelperIo>(io: &IO, args: &HelperArgs) -> i32 {
    let target = Path::new(&args.target);
    let new_path = PathBuf::from(format!("{}.new", args.target));
    let old_path = PathBuf::from(format!("{}.old", args.target));

    io.log(&format!("助手启动：目标={}", args.target));

    // ① 等旧进程退出；超时放弃（不下载、不动文件）
    if let Some(pid) = args.wait_pid {
        io.log(&format!("等待进程 {pid} 退出"));
        let mut polls = 0u32;
        while io.is_running(pid) {
            if polls >= WAIT_MAX_POLLS {
                io.log(&format!("等待进程 {pid} 退出超时，放弃更新"));
                return 1;
            }
            polls += 1;
            let ms = io.poll_interval_ms();
            if ms > 0 {
                std::thread::sleep(Duration::from_millis(ms));
            }
        }
    }

    // ② 下载
    let bytes = match io.download(&args.url) {
        Ok(b) => b,
        Err(e) => {
            io.log(&format!("下载失败：{e}"));
            let _ = io.spawn(target);
            return 1;
        }
    };

    // ③ SHA256 校验：不过关绝不落盘
    let actual = sha256_hex(&bytes);
    if !actual.eq_ignore_ascii_case(args.sha256.trim()) {
        io.log(&format!("SHA256 不符：期望 {} 实际 {actual}", args.sha256));
        let _ = io.spawn(target);
        return 1;
    }

    // ④ 写临时文件（同目录，保证后续 rename 是同卷原子操作）
    if let Err(e) = io.write(&new_path, &bytes) {
        io.log(&format!("写入临时文件失败：{e}"));
        let _ = io.remove(&new_path);
        let _ = io.spawn(target);
        return 1;
    }

    // ⑤ 备份现有 exe
    if let Err(e) = io.rename(target, &old_path) {
        io.log(&format!("备份旧 exe 失败：{e}"));
        let _ = io.remove(&new_path);
        let _ = io.spawn(target);
        return 1;
    }

    // ⑥ 顶替；失败则回滚并把旧版拉起来
    if let Err(e) = io.rename(&new_path, target) {
        io.log(&format!("替换失败，尝试回滚：{e}"));
        let _ = io.remove(&new_path);
        if let Err(e2) = io.rename(&old_path, target) {
            io.log(&format!("回滚失败，请手动把 {old_path:?} 改回 {target:?}：{e2}"));
            return 1;
        }
        if let Err(e3) = io.spawn(target) {
            io.log(&format!("回滚后拉起旧版失败（请手动启动）：{e3}"));
        }
        return 1;
    }

    // 拉起失败时恢复旧文件，不把无法执行的新包当作成功。
    if let Err(e) = io.spawn(target) {
        io.log(&format!("拉起新版失败，回滚：{e}"));
        if io.remove(target).is_ok() && io.rename(&old_path, target).is_ok() {
            let _ = io.spawn(target);
        }
        return 1;
    }

    // ⑧ 清理备份
    let _ = io.remove(&old_path);
    io.log("更新完成");
    0
}

/// 计算字节的 SHA256 小写十六进制
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// 真实 IO 实现：进程探测复用 `win::process_exe_path`，下载走 reqwest blocking，
/// 日志追加写 exe 同目录 `update-helper.log`。
pub struct RealHelperIo {
    log_path: PathBuf,
}

impl RealHelperIo {
    pub fn new() -> Self {
        let dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            log_path: dir.join("update-helper.log"),
        }
    }
}

impl Default for RealHelperIo {
    fn default() -> Self {
        Self::new()
    }
}

impl HelperIo for RealHelperIo {
    fn is_running(&self, pid: u32) -> bool {
        // 进程退出后句柄打不开 → None；这是能查到「已退出」的最轻量判据
        crate::win::process_exe_path(pid).is_some()
    }

    fn download(&self, url: &str) -> Result<Vec<u8>, String> {
        let resp = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| e.to_string())?
            .get(url)
            .send()
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        resp.bytes().map(|b| b.to_vec()).map_err(|e| e.to_string())
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        std::fs::write(path, bytes).map_err(|e| e.to_string())
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<(), String> {
        // Windows 上 rename 不覆盖已存在目标，先删
        if to.exists() {
            let _ = std::fs::remove_file(to);
        }
        std::fs::rename(from, to).map_err(|e| e.to_string())
    }

    fn remove(&self, path: &Path) -> Result<(), String> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }

    fn spawn(&self, path: &Path) -> Result<(), String> {
        std::process::Command::new(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn log(&self, msg: &str) {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = writeln!(f, "[update-helper] {msg}");
        }
    }

    fn poll_interval_ms(&self) -> u64 {
        500
    }
}

/// 以真实 IO 跑助手流程（`fn main` 的 `--apply-update` 分支调用）。
pub fn run_helper(args: HelperArgs) -> i32 {
    let Ok(_ownership) = helper_lock(Path::new(&args.target)) else { return 1; };
    run_helper_flow(&RealHelperIo::new(), &args)
}

// ---- 自动更新：远端清单 / 安装版判定 / 手动与自动检查（v1.4.0 阶段 A）----

/// 远端更新清单地址（GitHub Release 的固定「最新」别名，与 tauri.conf.json 的 endpoints 同值）。
pub const DEFAULT_ENDPOINT: &str =
    "https://github.com/wangdxnum1/niuma-timer/releases/latest/download/latest.json";

/// 绿色版导出的平台键（A9 生成 latest.json 时使用同名键）。
pub const PORTABLE_KEY: &str = "windows-x86_64-portable";

/// 安装版（NSIS）的平台键。
pub const NSIS_KEY: &str = "windows-x86_64";

/// `check_update` 的返回体（前端「更新」页直接渲染）。
#[derive(Serialize, Clone, Debug)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub has_update: bool,
    pub notes: Option<String>,
    pub installed: bool,
    pub error: Option<String>,
}

fn installation_matches(exe: &Path, location: &str, icon: &str) -> bool {
    let norm = |s: &str| s.trim().trim_matches('"').replace('/', "\\").trim_end_matches('\\').to_lowercase();
    let parent = exe.parent().map(|p| norm(&p.to_string_lossy())).unwrap_or_default();
    if !location.trim().is_empty() && norm(location) == parent { return true; }
    let icon_path = if icon.starts_with('"') {
        icon[1..].split('"').next().unwrap_or("")
    } else { icon.rsplit_once(',').map(|(p, _)| p).unwrap_or(icon) };
    !icon_path.is_empty() && norm(icon_path) == norm(&exe.to_string_lossy())
}

/// Match the actual running path across user/machine and both registry views.
pub fn installed_kind() -> Option<&'static str> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY};
    use winreg::RegKey;
    let exe = std::env::current_exe().ok()?;
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            let root = RegKey::predef(hive);
            let Ok(uninstall) = root.open_subkey_with_flags(r"Software\Microsoft\Windows\CurrentVersion\Uninstall", KEY_READ | view) else { continue };
            for name in uninstall.enum_keys().flatten() {
                let Ok(sub) = uninstall.open_subkey(&name) else { continue };
                let display: String = sub.get_value("DisplayName").unwrap_or_default();
                if !display.contains("牛马计时器") && !display.contains("niuma-timer") { continue; }
                let location: String = sub.get_value("InstallLocation").unwrap_or_default();
                let icon: String = sub.get_value("DisplayIcon").unwrap_or_default();
                if !installation_matches(&exe, &location, &icon) { continue; }
                let msi: u32 = sub.get_value("WindowsInstaller").unwrap_or(0);
                let uninstall: String = sub.get_value("UninstallString").unwrap_or_default();
                return Some(if msi == 1 || uninstall.to_lowercase().contains("msiexec") {
                    "windows-x86_64-msi"
                } else { "windows-x86_64-nsis" });
            }
        }
    }
    None
}

pub fn is_installed() -> bool { installed_kind().is_some() }

/// 本轮所有网络请求的唯一入口（15 秒超时 + 显式 UA：GitHub 对无 UA 请求会限流）。
pub fn fetch_text(url: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("niuma-timer/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(url).send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.text().map_err(|e| e.to_string())
}

/// 拉取并解析远端清单。
pub fn fetch_remote() -> Result<RemoteRelease, String> {
    let text = fetch_text(DEFAULT_ENDPOINT)?;
    parse_latest_json(&text)
}

/// 由 latest.json 的地址推出同目录的 SHA256SUMS.txt 地址。
pub fn sums_url_for(latest_url: &str) -> String {
    match latest_url.rsplit_once('/') {
        Some((dir, _)) => format!("{dir}/SHA256SUMS.txt"),
        None => latest_url.to_string(),
    }
}

/// 在 SHA256SUMS.txt 里按**完整文件名**找哈希（前缀相同不算命中，防止拿错包）。
/// 只认 64 位十六进制，避免把残缺行当成有效哈希。
pub fn find_hash_in_sums(sums: &str, file_name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut it = line.split_whitespace();
        let hash = it.next()?;
        let name = it.next()?;
        if name == file_name && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            Some(hash.to_string())
        } else {
            None
        }
    })
}

/// 组装助手参数：下载地址里的文件名 → 去 SHA256SUMS.txt 取期望哈希。
pub fn build_helper_args(
    target: &str,
    wait_pid: u32,
    url: &str,
    sums: &str,
) -> Result<HelperArgs, String> {
    let file_name = url.rsplit('/').next().unwrap_or("");
    if file_name.is_empty() {
        return Err("下载地址里没有文件名".to_string());
    }
    let sha256 = find_hash_in_sums(sums, file_name)
        .ok_or_else(|| format!("SHA256SUMS.txt 里没有 {file_name} 的哈希"))?;
    Ok(HelperArgs {
        target: target.to_string(),
        wait_pid: Some(wait_pid),
        url: url.to_string(),
        sha256,
    })
}

/// 是否「刚升级」：上次记录的运行版本存在且与当前版本不同。
/// `None` 表示全新安装（没有上一个版本可比），不算升级，不该弹更新公告。
pub fn is_just_upgraded(last_run: Option<&str>, current: &str) -> bool {
    last_run.map(|v| v != current).unwrap_or(false)
}

/// 编译期从 CHANGELOG 抽出的当前版本说明（`build.rs` 把真换行转义成字面 `\n`）。
pub fn update_notes() -> String {
    env!("UPDATE_NOTES").replace("\\n", "\n")
}

/// 取本次更新公告并**落盘记账**：无论是否返回内容，都把当前版本写进配置，
/// 保证同一版本只弹一次。返回 `None` 的三种情况：全新安装、已展示过、CHANGELOG 没抽到内容。
pub fn take_announcement(app: &tauri::AppHandle) -> Option<String> {
    let current = env!("CARGO_PKG_VERSION");
    let state = app.state::<crate::AppState>();
    let mut cfg = crate::sync::lock(&state.config, "state.config");
    let changed = is_just_upgraded(cfg.update_last_run_version.as_deref(), current);
    cfg.update_last_run_version = Some(current.to_string());
    crate::config::save(&cfg);
    if !changed {
        return None;
    }
    let notes = update_notes();
    if notes.trim().is_empty() {
        None
    } else {
        Some(notes)
    }
}

/// 手动/自动检查共用的取数逻辑：只返回远端与本地版本的比较结果，不施加任何跳过过滤。
/// 网络异常不 panic，转成 `error` 字段交给前端展示（手动检查要看得见失败原因）。
pub fn check_now() -> UpdateInfo {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let installed = is_installed();
    match fetch_remote() {
        Ok(rel) => {
            let has_update = compare_versions(&rel.version, &current) == Ordering::Greater;
            let notes = if has_update { rel.notes } else { None };
            UpdateInfo {
                current,
                latest: rel.version,
                has_update,
                notes,
                installed,
                error: None,
            }
        }
        Err(e) => {
            crate::db::debug_log(&format!("检查更新失败：{e}"));
            UpdateInfo {
                latest: current.clone(),
                current,
                has_update: false,
                notes: None,
                installed,
                error: Some(e),
            }
        }
    }
}

/// 自动检查：发现可用且未被跳过的版本才提示（写托盘项 + 发系统通知）。
/// 返回被提示的版本号，供调用方记日志。
pub fn check_and_notify(app: &tauri::AppHandle) -> Option<String> {
    let rel = match fetch_remote() {
        Ok(r) => r,
        Err(e) => {
            // 自动检查失败静默：只落调试日志，不打扰用户
            crate::db::debug_log(&format!("自动检查更新失败：{e}"));
            return None;
        }
    };
    let skipped = {
        let state = app.state::<crate::AppState>();
        let mut cfg = crate::sync::lock(&state.config, "state.config");
        cfg.update_last_check = Some(chrono::Local::now().to_rfc3339());
        crate::config::save(&cfg);
        cfg.update_skipped_version.clone()
    };
    let current = env!("CARGO_PKG_VERSION");
    if !should_notify(&rel.version, current, skipped.as_deref()) {
        return None;
    }
    crate::tray::set_update_available(app, &rel.version);
    crate::remind::notify(
        app,
        "发现新版本",
        &format!("牛马计时器 v{} 已发布，可一键更新", rel.version),
    );
    Some(rel.version)
}

/// 每次等待的随机抖动（0–899 秒）：避免多台机器开机后同一秒一起打 GitHub。
fn jitter_secs() -> u64 {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    u64::from(d.subsec_nanos() % 900)
}

/// 启动自动检查线程：延迟 30 秒先跑一次（不抢启动期资源），
/// 之后每 6 小时 ± 抖动一次。每轮开头重读开关，用户关掉后下一轮即生效。
pub fn spawn_update_checker(app: tauri::AppHandle) {
    let _ = std::thread::Builder::new()
        .name("niuma-update-check".to_string())
        .spawn(move || {
            std::thread::sleep(Duration::from_secs(30));
            loop {
                let enabled = {
                    let state = app.state::<crate::AppState>();
                    let cfg = crate::sync::lock(&state.config, "state.config");
                    cfg.update_auto_check
                };
                if enabled {
                    check_and_notify(&app);
                }
                std::thread::sleep(Duration::from_secs(6 * 3600 + jitter_secs()));
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installation_must_match_running_executable() {
        let exe = Path::new(r"C:\Program Files\Niuma\niuma-timer.exe");
        assert!(installation_matches(exe, r"C:\Program Files\Niuma\", ""));
        assert!(installation_matches(exe, "", r#""C:\Program Files\Niuma\niuma-timer.exe",0"#));
        assert!(!installation_matches(Path::new(r"C:\Portable\niuma-timer.exe"), r"C:\Program Files\Niuma", ""));
        assert!(!installation_matches(exe, "", ""));
    }
    #[test]
    fn is_just_upgraded_paths() {
        // None = 全新安装（没有上一个版本可比），不该弹公告
        assert!(!is_just_upgraded(None, "1.4.0"));
        // 同一版本 = 已展示过
        assert!(!is_just_upgraded(Some("1.4.0"), "1.4.0"));
        // 版本变化 = 刚升级
        assert!(is_just_upgraded(Some("1.3.0"), "1.4.0"));
    }

    #[test]
    fn find_hash_in_sums_exact_name_only() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let sums =
            format!("{a}  niuma-timer-1.4.0-portable.exe\n{b}  niuma-timer-1.4.0-setup.exe\n");
        assert_eq!(
            find_hash_in_sums(&sums, "niuma-timer-1.4.0-portable.exe"),
            Some(a)
        );
        // 前缀相同但文件名不完整 → 不命中（防止拿错包）
        assert_eq!(find_hash_in_sums(&sums, "niuma-timer-1.4.0-portable"), None);
        // 位数不足的哈希视为无效行
        assert_eq!(find_hash_in_sums("deadbeef  x.exe\n", "x.exe"), None);
    }
    use std::cell::{Cell, RefCell};
    /// 假 IO：记录调用序列，可注入「下载内容 / 某次重命名失败 / 进程永不退出」。
    /// 手写 `Default`——`Result` 没实现 `Default`，不能 derive。
    struct FakeIo {
        /// download 返回值
        download_result: Result<Vec<u8>, String>,
        /// 第 N 次 rename（从 1 起）返回失败；None = 从不失败
        fail_rename_at: Option<u32>,
        fail_first_spawn: bool,
        /// true 时 is_running 恒为 true（模拟旧进程僵死）
        never_exit: bool,
        rename_calls: Cell<u32>,
        written: RefCell<Vec<PathBuf>>,
        removed: RefCell<Vec<PathBuf>>,
        spawned: RefCell<Vec<PathBuf>>,
        logged: RefCell<Vec<String>>,
    }

    impl Default for FakeIo {
        fn default() -> Self {
            Self {
                download_result: Ok(vec![1, 2, 3]),
                fail_rename_at: None,
                fail_first_spawn: false,
                never_exit: false,
                rename_calls: Cell::new(0),
                written: RefCell::new(Vec::new()),
                removed: RefCell::new(Vec::new()),
                spawned: RefCell::new(Vec::new()),
                logged: RefCell::new(Vec::new()),
            }
        }
    }

    impl HelperIo for FakeIo {
        fn is_running(&self, _pid: u32) -> bool {
            self.never_exit
        }

        fn download(&self, _url: &str) -> Result<Vec<u8>, String> {
            self.download_result.clone()
        }

        fn write(&self, path: &Path, _bytes: &[u8]) -> Result<(), String> {
            self.written.borrow_mut().push(path.to_path_buf());
            Ok(())
        }

        fn rename(&self, _from: &Path, _to: &Path) -> Result<(), String> {
            let n = self.rename_calls.get() + 1;
            self.rename_calls.set(n);
            if self.fail_rename_at == Some(n) {
                return Err(format!("模拟第 {n} 次 rename 失败"));
            }
            Ok(())
        }

        fn remove(&self, path: &Path) -> Result<(), String> {
            self.removed.borrow_mut().push(path.to_path_buf());
            Ok(())
        }

        fn spawn(&self, path: &Path) -> Result<(), String> {
            self.spawned.borrow_mut().push(path.to_path_buf());
            if self.fail_first_spawn && self.spawned.borrow().len() == 1 { return Err("spawn failed".to_string()); }
            Ok(())
        }

        fn log(&self, msg: &str) {
            self.logged.borrow_mut().push(msg.to_string());
        }

        fn poll_interval_ms(&self) -> u64 {
            0
        }
    }

    /// 与 FakeIo 默认下载内容 `[1, 2, 3]` 哈希对齐的助手参数
    fn helper_args_for_test() -> HelperArgs {
        HelperArgs {
            target: "C:\\app\\niuma-timer.exe".to_string(),
            wait_pid: Some(1),
            url: "https://example.com/new.exe".to_string(),
            sha256: sha256_hex(&[1, 2, 3]),
        }
    }

    #[test]
    fn run_helper_flow_success() {
        let io = FakeIo::default();
        let args = helper_args_for_test();
        assert_eq!(run_helper_flow(&io, &args), 0);
        // 写临时文件 → 备份 → 顶替 → 拉新 → 清理备份
        assert_eq!(io.written.borrow().len(), 1);
        assert!(io.written.borrow()[0].ends_with("niuma-timer.exe.new"));
        assert_eq!(io.rename_calls.get(), 2);
        assert_eq!(
            *io.spawned.borrow(),
            vec![PathBuf::from("C:\\app\\niuma-timer.exe")]
        );
        assert_eq!(
            *io.removed.borrow(),
            vec![PathBuf::from("C:\\app\\niuma-timer.exe.old")]
        );
    }

    #[test]
    fn run_helper_flow_hash_mismatch() {
        let io = FakeIo {
            download_result: Ok(vec![9, 9, 9]),
            ..FakeIo::default()
        };
        let args = helper_args_for_test();
        assert_eq!(run_helper_flow(&io, &args), 1);
        // 校验不过关：绝不写盘，更不动旧 exe
        assert!(io.written.borrow().is_empty());
        assert_eq!(io.rename_calls.get(), 0);
        assert_eq!(io.spawned.borrow().len(), 1, "校验失败应重新拉起旧版");
    }

    #[test]
    fn run_helper_flow_spawn_failure_rolls_back() {
        let io = FakeIo { fail_first_spawn: true, ..FakeIo::default() };
        assert_eq!(run_helper_flow(&io, &helper_args_for_test()), 1);
        assert_eq!(io.rename_calls.get(), 3);
        assert_eq!(io.spawned.borrow().len(), 2);
    }

    #[test]
    fn run_helper_flow_download_failure_restarts_old() {
        let io = FakeIo { download_result: Err("offline".to_string()), ..FakeIo::default() };
        assert_eq!(run_helper_flow(&io, &helper_args_for_test()), 1);
        assert_eq!(io.spawned.borrow().len(), 1);
        assert_eq!(io.rename_calls.get(), 0);
    }

    #[test]
    fn run_helper_flow_wait_timeout() {
        let io = FakeIo {
            never_exit: true,
            ..FakeIo::default()
        };
        let args = helper_args_for_test();
        assert_eq!(run_helper_flow(&io, &args), 1);
        assert_eq!(io.rename_calls.get(), 0);
        assert!(io.spawned.borrow().is_empty());
        assert!(io.logged.borrow().iter().any(|m| m.contains("超时")));
    }

    #[test]
    fn run_helper_flow_rollback_on_second_rename() {
        // 第 2 次 rename（.new → target）失败 → 第 3 次 rename 回滚成功 → 重新拉起旧版
        let io = FakeIo {
            fail_rename_at: Some(2),
            ..FakeIo::default()
        };
        let args = helper_args_for_test();
        assert_eq!(run_helper_flow(&io, &args), 1);
        assert_eq!(io.rename_calls.get(), 3);
        assert_eq!(
            *io.spawned.borrow(),
            vec![PathBuf::from("C:\\app\\niuma-timer.exe")]
        );
    }

    /// 构造一个只带版本号的 RemoteRelease
    fn rel(v: &str) -> RemoteRelease {
        RemoteRelease {
            version: v.to_string(),
            notes: None,
            pub_date: None,
            platforms: HashMap::new(),
        }
    }

    /// 构造一条最小可用的 latest.json 文本
    fn latest(v: &str, notes: Option<&str>) -> String {
        let notes_field = match notes {
            Some(n) => format!(r#""notes":"{n}","#),
            None => String::new(),
        };
        format!(
            r#"{{"version":"{v}",{notes_field}"platforms":{{"windows-x86_64":{{"signature":"sig","url":"https://example.com/a.exe"}}}}}}"#
        )
    }

    #[test]
    fn compare_versions_numeric_order() {
        assert_eq!(compare_versions("1.10.0", "1.9.0"), Ordering::Greater);
    }

    #[test]
    fn compare_versions_uneven_length() {
        assert_eq!(compare_versions("1.4", "1.4.0"), Ordering::Equal);
    }

    #[test]
    fn compare_versions_prerelease() {
        assert_eq!(compare_versions("1.4.0-beta.1", "1.4.0"), Ordering::Less);
    }

    #[test]
    fn compare_versions_patch() {
        assert_eq!(compare_versions("1.4.1", "1.4.0"), Ordering::Greater);
    }

    #[test]
    fn parse_latest_json_ok() {
        let r = parse_latest_json(&latest("1.4.0", Some("更新说明"))).unwrap();
        assert_eq!(r.version, "1.4.0");
        assert_eq!(r.notes.as_deref(), Some("更新说明"));
        assert_eq!(
            r.platform_url("windows-x86_64"),
            Some("https://example.com/a.exe")
        );
    }

    #[test]
    fn parse_latest_json_missing_version() {
        assert!(parse_latest_json(r#"{"platforms":{}}"#).is_err());
    }

    #[test]
    fn parse_latest_json_bad_json() {
        assert!(parse_latest_json("not a json").is_err());
    }

    #[test]
    fn parse_latest_json_notes_optional() {
        let r = parse_latest_json(&latest("1.4.0", None)).unwrap();
        assert_eq!(r.notes, None);
    }

    #[test]
    fn remote_release_platform_url_miss() {
        assert_eq!(rel("1.4.0").platform_url("windows-x86_64"), None);
    }

    #[test]
    fn should_notify_same_version() {
        assert!(!should_notify("1.4.0", "1.4.0", None));
    }

    #[test]
    fn should_notify_remote_older() {
        assert!(!should_notify("1.3.0", "1.4.0", None));
    }

    #[test]
    fn should_notify_newer() {
        assert!(should_notify("1.5.0", "1.4.0", None));
    }

    #[test]
    fn should_notify_skipped_same() {
        assert!(!should_notify("1.5.0", "1.4.0", Some("1.5.0")));
    }

    #[test]
    fn should_notify_newer_than_skipped() {
        assert!(should_notify("1.6.0", "1.4.0", Some("1.5.0")));
    }

    #[test]
    fn parse_helper_args_full() {
        let argv: Vec<String> = [
            "niuma-timer.exe",
            "--apply-update",
            "--target",
            "C:\\app\\niuma-timer.exe",
            "--wait-pid",
            "1234",
            "--url",
            "https://example.com/new.exe",
            "--sha256",
            "abc",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let h = parse_helper_args(&argv).unwrap();
        assert_eq!(h.target, "C:\\app\\niuma-timer.exe");
        assert_eq!(h.wait_pid, Some(1234));
        assert_eq!(h.url, "https://example.com/new.exe");
        assert_eq!(h.sha256, "abc");
    }

    #[test]
    fn parse_helper_args_no_flag() {
        let argv: Vec<String> = ["niuma-timer.exe", "--minimized"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(parse_helper_args(&argv).is_none());
    }

    #[test]
    fn parse_helper_args_missing_required() {
        let argv: Vec<String> = ["niuma-timer.exe", "--apply-update", "--target", "x.exe"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(parse_helper_args(&argv).is_none());
    }
}
