//! 存储与备份命令：存储占用快照、手动维护、备份创建/列表/还原。
//!
//! 二期自 main.rs 切出（纯移动）。

use tauri::{AppHandle, Manager, State};

use crate::backup;
use crate::maintain;
use crate::state::AppState;
use crate::sync;

/// 存储占用快照：设置页「数据存储」卡片展示用
///
/// `(async)`：同步 command 默认在**主线程**执行，而本命令要做 dbstat 全库扫描、
/// 7 张表 COUNT 与目录遍历；下面的 run_maintenance 更是持全局 DB 锁做 WAL
/// TRUNCATE + 数据清理，最坏秒级——期间托盘 1s 刷新与窗口/托盘事件全部冻结。
/// 标 async 后函数体改在异步运行时线程执行，主线程只管立即响应。
#[tauri::command(async)]
pub(crate) fn get_storage_info(
    state: State<'_, AppState>,
) -> Result<maintain::StorageInfo, String> {
    let cfg = sync::lock(&state.config, "state.config").clone();
    Ok(maintain::storage_info(&cfg))
}

/// 立即执行一次维护（WAL 收缩 + 过期图标 + 过期数据），返回执行后的占用快照。
/// 与调度器每日自动跑的是同一套逻辑，用户点按钮只是提前触发。
/// `(async)` 理由同 get_storage_info：持 DB 锁的重活不能占主线程。
#[tauri::command(async)]
pub(crate) fn run_maintenance(state: State<'_, AppState>) -> Result<maintain::StorageInfo, String> {
    let cfg = sync::lock(&state.config, "state.config").clone();
    maintain::run_daily(&cfg);
    Ok(maintain::storage_info(&cfg))
}

#[tauri::command(async)]
pub(crate) fn backup_now(app: tauri::AppHandle) -> Result<backup::BackupEntry, String> {
    let state = app.state::<AppState>();
    let _cfg = sync::lock(&state.config, "state.config");
    backup::create_backup(env!("CARGO_PKG_VERSION"))
}

#[tauri::command(async)]
pub(crate) fn list_backups() -> Vec<backup::BackupEntry> {
    backup::list_backups()
}

#[tauri::command(async)]
pub(crate) fn delete_backup(name: String) -> Result<(), String> {
    backup::delete_backup(&name)
}

#[tauri::command(async)]
pub(crate) fn restore_backup(app: AppHandle, name: String) -> Result<String, String> {
    let summary = {
        let state = app.state::<AppState>();
        let _cfg = sync::lock(&state.config, "state.config");
        backup::stage_restore(&name, env!("CARGO_PKG_VERSION"))?
    };
    let handle = app.clone();
    std::thread::Builder::new()
        .name("niuma-restore-restart".to_string())
        .spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(800));
            handle.restart();
        })
        .map_err(|e| format!("还原已准备好，请手动重启（自动重启失败：{e}）"))?;
    Ok(summary)
}
