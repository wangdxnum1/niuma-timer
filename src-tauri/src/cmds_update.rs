//! 更新命令：检查更新、立即更新（安装版 / 绿色版两条路径）、跳过版本、更新公告。
//!
//! 二期自 main.rs 切出（纯移动）：start_update_installed / start_update_portable
//! 本就是 update.rs 的下载-校验-交接流程接线，物理放回更新域旁边。

use tauri::Manager;
use tauri_plugin_updater::UpdaterExt;

use crate::config;
use crate::state::AppState;
use crate::sync;
use crate::update;

/// 检查更新（手动入口）。网络 IO 交给 spawn_blocking，不占主线程；
/// 返回原始真相、不受「跳过此版本」影响（手动检查不该被 skip 掩盖）。
#[tauri::command]
pub(crate) async fn check_update(app: tauri::AppHandle) -> Result<update::UpdateInfo, String> {
    let info = tauri::async_runtime::spawn_blocking(update::check_now)
        .await
        .map_err(|e| e.to_string())?;
    if info.error.is_none() {
        let state = app.state::<AppState>();
        let mut cfg = sync::lock(&state.config, "state.config");
        cfg.update_last_check = Some(chrono::Local::now().to_rfc3339());
        // 时间戳性质的记账，落盘失败由 save 记 debug.log，检查结果照常返回
        let _ = config::save(&cfg);
    }
    Ok(info)
}

/// 立即更新：按安装形态分发；两条路径都把进度经 `update-progress` 事件推给前端。
#[tauri::command]
pub(crate) async fn start_update(app: tauri::AppHandle) -> Result<(), String> {
    let _operation = update::UpdateOperation::begin()?;
    if update::installed_kind().is_some() {
        start_update_installed(app).await
    } else {
        start_update_portable(app).await
    }
}

/// 安装版：官方 updater 分步下载（带进度与重试）→ 交给安装器静默安装。
/// 成功后进程由安装器接管，`install` 返回即到头——这里不也不该再 restart。
async fn start_update_installed(app: tauri::AppHandle) -> Result<(), String> {
    let Some(target) = update::installed_kind() else {
        return Err("当前不是安装版".to_string());
    };
    let updater = app
        .updater_builder()
        .target(target)
        .build()
        .map_err(|e| e.to_string())?;
    let Some(pending) = updater.check().await.map_err(|e| e.to_string())? else {
        return Err("已是最新版本".to_string());
    };
    update::emit_progress(&app, "downloading", 0, None, 1);
    let mut bytes: Option<Vec<u8>> = None;
    let mut last_err = String::from("未知错误");
    for attempt in 1..=update::DOWNLOAD_ATTEMPTS {
        // Cell 不是 Send：跨 await 的闭包只能捕获 owned 局部量
        let mut seen: u64 = 0;
        let mut last_pct: i64 = -2;
        let mut last_at = std::time::Instant::now();
        let chunk_app = app.clone();
        let on_chunk = move |len: usize, total: Option<u64>| {
            seen += len as u64;
            let pct = match total {
                Some(t) if t > 0 => (seen as f64 / t as f64 * 100.0) as i64,
                _ => -1,
            };
            let now = std::time::Instant::now();
            // 节流：百分比变化或距上次 ≥120ms 才发；total 未知时只按时间节流
            if pct != last_pct
                || now.duration_since(last_at) >= std::time::Duration::from_millis(120)
            {
                last_pct = pct;
                last_at = now;
                update::emit_progress(&chunk_app, "downloading", seen, total, attempt);
            }
        };
        let finish_app = app.clone();
        let on_finish = move || update::emit_progress(&finish_app, "verifying", 0, None, attempt);
        match pending.download(on_chunk, on_finish).await {
            Ok(b) => {
                bytes = Some(b);
                break;
            }
            Err(e) => last_err = e.to_string(),
        }
        if attempt < update::DOWNLOAD_ATTEMPTS {
            update::emit_progress(&app, "retrying", 0, None, attempt + 1);
            // async 环境里的退避等待：丢到阻塞线程再 sleep
            let secs = if attempt <= 1 { 1u64 } else { 3u64 };
            let _ = tauri::async_runtime::spawn_blocking(move || {
                std::thread::sleep(std::time::Duration::from_secs(secs))
            })
            .await;
        }
    }
    let Some(bytes) = bytes else {
        update::emit_progress(&app, "error", 0, None, update::DOWNLOAD_ATTEMPTS);
        return Err(format!(
            "下载失败（已重试 {} 次）：{last_err}",
            update::DOWNLOAD_ATTEMPTS - 1
        ));
    };
    update::emit_progress(
        &app,
        "installing",
        bytes.len() as u64,
        Some(bytes.len() as u64),
        1,
    );
    pending.install(&bytes).map_err(|e| e.to_string())?;
    Ok(())
}

/// 绿色版：主程序带进度下载 → SHA256 校验 → 落盘交接文件 → 起内置助手复核替换。
/// 下载从助手提前到主程序，才能把真实进度发给前端；助手只做「复核 + 原子替换」。
async fn start_update_portable(app: tauri::AppHandle) -> Result<(), String> {
    let rel = tauri::async_runtime::spawn_blocking(update::fetch_remote)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let url = rel
        .platform_url(update::PORTABLE_KEY)
        .ok_or_else(|| "远端清单里没有便携包条目".to_string())?
        .to_string();
    if update::compare_versions(&rel.version, env!("CARGO_PKG_VERSION"))
        != std::cmp::Ordering::Greater
    {
        return Err("已是最新版本".to_string());
    }
    let sums_url = update::sums_url_for(&url);
    let sums = tauri::async_runtime::spawn_blocking(move || update::fetch_text_fallback(&sums_url))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let file_name = url.rsplit('/').next().unwrap_or("");
    if file_name.is_empty() {
        return Err("下载地址里没有文件名".to_string());
    }
    let expected = update::find_hash_in_sums(&sums, file_name)
        .ok_or_else(|| format!("SHA256SUMS.txt 里没有 {file_name} 的哈希"))?;

    // 下载 + 重试都在阻塞线程里跑（不卡 async 运行时）；AppHandle 先克隆好分给各闭包
    let progress_app = app.clone();
    let download_url = url.clone();
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        let downloading_app = progress_app.clone();
        let retry_app = progress_app;
        let mut last_pct: i64 = -2;
        let mut last_at = std::time::Instant::now();
        let fetcher = update::RealFetcher;
        update::download_with_retry(
            &fetcher,
            &download_url,
            &mut |done: u64, total: Option<u64>, attempt: u32| {
                let pct = match total {
                    Some(t) if t > 0 => (done as f64 / t as f64 * 100.0) as i64,
                    _ => -1,
                };
                let now = std::time::Instant::now();
                if pct != last_pct
                    || now.duration_since(last_at) >= std::time::Duration::from_millis(120)
                {
                    last_pct = pct;
                    last_at = now;
                    update::emit_progress(&downloading_app, "downloading", done, total, attempt);
                }
            },
            &mut |attempt: u32, _err: &str| {
                update::emit_progress(&retry_app, "retrying", 0, None, attempt);
            },
            &mut |attempt: u32| {
                let secs = if attempt <= 1 { 1u64 } else { 3u64 };
                std::thread::sleep(std::time::Duration::from_secs(secs));
            },
        )
    })
    .await
    .map_err(|e| e.to_string())?
    .inspect_err(|_| {
        update::emit_progress(&app, "error", 0, None, update::DOWNLOAD_ATTEMPTS);
    })?;

    update::emit_progress(
        &app,
        "verifying",
        bytes.len() as u64,
        Some(bytes.len() as u64),
        1,
    );
    if !update::sha256_hex(&bytes).eq_ignore_ascii_case(&expected) {
        update::emit_progress(&app, "error", 0, None, 1);
        return Err(format!("SHA256 校验失败：期望 {expected}"));
    }

    // 校验通过才落盘交接文件（与 exe 同目录同卷，助手改名是原子操作）
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let staging = format!("{}.download", exe.to_string_lossy());
    std::fs::write(&staging, &bytes).map_err(|e| e.to_string())?;

    let args = update::build_helper_args_local(
        &exe.to_string_lossy(),
        std::process::id(),
        &staging,
        &expected,
    );
    let mut argv = vec![
        "--apply-update".to_string(),
        "--target".to_string(),
        args.target.clone(),
    ];
    if let Some(pid) = args.wait_pid {
        argv.push("--wait-pid".to_string());
        argv.push(pid.to_string());
    }
    let update::Source::Local(staging_path) = &args.source else {
        return Err("内部错误：交接参数不是本地文件来源".to_string());
    };
    argv.push("--local-file".to_string());
    argv.push(staging_path.clone());
    argv.push("--sha256".to_string());
    argv.push(args.sha256.clone());
    update::emit_progress(
        &app,
        "installing",
        bytes.len() as u64,
        Some(bytes.len() as u64),
        1,
    );
    std::process::Command::new(&exe)
        .args(&argv)
        .spawn()
        .map_err(|e| e.to_string())?;
    app.exit(0);
    Ok(())
}

/// 跳过某版本：之后不再提示该版本，直到远端出现更高的版本。
/// 只写配置不清托盘（重启后托盘项自然消失）—— spec §11.3 的验收口径。
#[tauri::command(async)]
pub(crate) fn skip_update_version(app: tauri::AppHandle, version: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut cfg = sync::lock(&state.config, "state.config");
    cfg.update_skipped_version = Some(version);
    // 用户显式操作：写不进去要让前端知道，不能弹了「已跳过」重启后又冒出来
    config::save(&cfg)?;
    Ok(())
}

/// 取本次更新公告：仅「升级后的首次启动」返回 CHANGELOG 段落，其余返回 None。
#[tauri::command(async)]
pub(crate) fn take_update_announcement(app: tauri::AppHandle) -> Option<String> {
    update::take_announcement(&app)
}
