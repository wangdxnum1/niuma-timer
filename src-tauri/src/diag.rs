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

/// 追加一行启动阶段痕迹到 panic.log（每次启动先由 install_crash_log 清空重写）。
pub(crate) fn trace_startup(stage: &str) {
    let _ = std::fs::create_dir_all(config::config_dir());
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(panic_log_path())
    {
        let line = format!(
            "[{}] {stage}\n",
            Local::now().format("%Y-%m-%d %H:%M:%S%.3f")
        );
        let _ = f.write_all(line.as_bytes());
    }
}

/// 安装 panic hook：捕获 main() 启动期任何 panic，写 message + backtrace 到 panic.log，
/// 让「进程起来又退出」的场景可定位。必须在 main() 最开头调用。
pub(crate) fn install_crash_log() {
    let _ = std::fs::create_dir_all(config::config_dir());
    let _ = std::fs::write(
        panic_log_path(),
        format!("[start] {}\n", Local::now().format("%Y-%m-%d %H:%M:%S%.3f")),
    );
    panic::set_hook(Box::new(|info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let line = format!("[panic] {info}\n{bt:?}\n");
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(panic_log_path())
            .and_then(|mut f| f.write_all(line.as_bytes()));
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
