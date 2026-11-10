//! 启动诊断：panic.log / 启动阶段痕迹 / 构建信息 / 致命错误弹窗。
//!
//! 二期自 main.rs 切出（纯移动）：这一组都围绕「进程早期可见性」——
//! webview 未起、release 无控制台时，这里是唯一的排查线索。

use std::io::Write;
use std::panic;

use chrono::Local;

use crate::config;

/// 启动崩溃诊断：进程早期（webview 未起）崩溃时前端 debug.log 无效，
/// 故把 panic 与启动阶段痕迹单独写到 %APPDATA%/niuma-timer/panic.log。
/// release 无控制台窗口，panic 会静默退出，此文件是排查「双击无反应」的唯一线索。
fn panic_log_path() -> std::path::PathBuf {
    config::config_dir().join("panic.log")
}

/// 兜底日志：%TEMP%/niuma-startup.log。
///
/// %APPDATA% 在受限上下文（沙盒进程、被安全软件接管、漫游配置异常）下可能不可写，
/// 而 panic.log 的写入失败是静默的——「双击无反应且没有任何日志」就是这么来的。
/// %TEMP% 对当前用户始终可写，两处同时留痕，才能区分「没进入 main」和「日志写不进去」。
fn temp_log_path() -> std::path::PathBuf {
    std::env::temp_dir().join("niuma-startup.log")
}

/// 程序所在目录的兜底日志 niuma-boot.log。
///
/// 该目录与 exe 同权限，正常安装/绿色版位置一定可写；用于区分「进程没进 main」
/// 与「%APPDATA%/%TEMP% 写不进去」——两者现象完全相同，只能靠第三处落点分辨。
fn exe_dir_log_path() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("niuma-boot.log")))
        .unwrap_or_else(|| std::path::PathBuf::from("niuma-boot.log"))
}

fn log_paths() -> Vec<std::path::PathBuf> {
    vec![panic_log_path(), temp_log_path(), exe_dir_log_path()]
}

/// 以追加方式写一行到指定文件，失败静默（诊断日志不能反过来影响启动）。
fn append_line(path: &std::path::Path, line: &str) {
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| f.write_all(line.as_bytes()));
}

/// 追加一行启动阶段痕迹（每次启动先由 install_crash_log 清空重写）。
pub(crate) fn trace_startup(stage: &str) {
    let _ = std::fs::create_dir_all(config::config_dir());
    let line = format!(
        "[{}] {stage}\n",
        Local::now().format("%Y-%m-%d %H:%M:%S%.3f")
    );
    for path in log_paths() {
        append_line(&path, &line);
    }
}

/// 安装 panic hook：捕获 main() 启动期任何 panic，写 message + backtrace 到 panic.log，
/// 让「进程起来又退出」的场景可定位。必须在 main() 最开头调用。
pub(crate) fn install_crash_log() {
    let _ = std::fs::create_dir_all(config::config_dir());
    let start = format!("[start] {}\n", Local::now().format("%Y-%m-%d %H:%M:%S%.3f"));
    let paths = log_paths();
    for path in &paths {
        let _ = std::fs::write(path, &start);
    }
    panic::set_hook(Box::new(move |info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let line = format!("[panic] {info}\n{bt:?}\n");
        for path in &paths {
            append_line(path, &line);
        }
    }));
}

/// 构建信息：版本 / 编译时间 / git 提交 / 前端指纹。
///
/// 同版本号可以构建很多次，光看 `v1.2.0` 分不清用户装的是哪一次构建、哪个提交、
/// 哪份前端资源。这四项由 build.rs 在编译期注入，用户报问题先看启动日志这一行即可对号入座。
/// 用 `option_env!` 兜底 `unknown`：源码包（无 .git / 无注入）也要能正常启动。
pub(crate) fn build_info() -> String {
    format!(
        "v{} build={} git={} fe={}",
        env!("CARGO_PKG_VERSION"),
        option_env!("BUILD_TIME").unwrap_or("unknown"),
        option_env!("BUILD_GIT").unwrap_or("unknown"),
        option_env!("BUILD_FE_VER").unwrap_or("unknown"),
    )
}

/// 启动致命错误：弹系统消息框（release 无控制台，必须给可见反馈），同时写 panic.log。
/// 把「双击无反应 / 静默退出」转成可操作的错误提示（尤其是缺 WebView2 的场景）。
#[cfg(windows)]
pub(crate) fn show_fatal(msg: &str) {
    let _ = std::fs::create_dir_all(config::config_dir());
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(panic_log_path())
    {
        let _ = f.write_all(format!("[fatal] {msg}\n").as_bytes());
    }
    crate::win::message_box("牛马计时器", msg);
}
