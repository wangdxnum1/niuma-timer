//! 核心命令：配置读写、节假日刷新、状态快照、窗口控制、调试日志、
//! 开机自启、CSV 导出，以及监控开关与全局快捷键的应用。
//!
//! 二期自 main.rs 切出（纯移动）。apply_monitor_switches / apply_shortcuts
//! 同时被 main() 启动序列与 save_config 复用，故 pub(crate)。

use chrono::{Datelike, Local};
use serde_json::Value;
use tauri::{Manager, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::activity;
use crate::app_usage;
use crate::audio_usage;
use crate::calc;
use crate::config;
use crate::db;
use crate::holiday;
use crate::state::{current_monthly_workdays, get_status, AppState};
use crate::sync;
use crate::tray;

#[tauri::command]
pub(crate) fn load_config(state: State<AppState>) -> config::Config {
    sync::lock(&state.config, "state.config").clone()
}

#[tauri::command]
pub(crate) fn save_config(
    state: State<AppState>,
    app: tauri::AppHandle,
    cfg: Value,
) -> Result<(), String> {
    // 合并保存：以现有配置为基底，仅用前端传来的字段覆盖，保留前端未管理的字段
    // （如未来新增的后端字段），避免整份替换把未传字段重置成默认值。
    let merged = {
        let mut existing = sync::lock(&state.config, "state.config");
        let merged = config::merge_from_value(&existing, &cfg)?;
        config::save(&merged);
        *existing = merged.clone();
        merged
    };
    // 监控开关即时生效（关闭前先结算已累计的应用使用时长）
    apply_monitor_switches(&merged);
    // 全局快捷键即时生效（开关关闭 → 解注册；开启 → 重新注册）
    apply_shortcuts(&app, &merged);
    let st = get_status(state.inner());
    tray::update_tray(&app, &st);
    Ok(())
}

/// 把配置里的三个监控开关同步到各监控模块（启动时与保存配置后调用）。
/// app_usage 在关闭前先 tick() 结算一次，避免丢掉最后一段已使用时长。
pub(crate) fn apply_monitor_switches(cfg: &config::Config) {
    if !cfg.monitor_app_usage {
        app_usage::shutdown();
    }
    activity::set_enabled(cfg.monitor_activity);
    app_usage::set_enabled(cfg.monitor_app_usage);
    audio_usage::set_enabled(cfg.monitor_audio);
    // 应用使用白名单（开启后只统计名单内应用）
    app_usage::set_whitelist(cfg.app_whitelist_enabled, cfg.app_whitelist.clone());
    // 重开时把当前前台窗口立即纳入统计
    if cfg.monitor_app_usage {
        app_usage::refresh_foreground();
    }
}

/// 把配置里的快捷键开关同步到 global-shortcut 插件（启动时与保存配置后调用）。
/// 先全量解注册再按需注册，保证开关切换后状态一致。
/// 单个快捷键注册失败（被其他软件占用）只记日志、不中断——快捷键是锦上添花，
/// 不能因为冲突让设置保存或启动失败。
pub(crate) fn apply_shortcuts(app: &tauri::AppHandle, cfg: &config::Config) {
    let mgr = app.global_shortcut();
    let _ = mgr.unregister_all();
    if !cfg.shortcuts_enabled {
        return;
    }
    // Alt+Shift+N：显隐主窗（可见 → 隐藏；否则还原 + 显示 + 抢焦点）
    if let Err(e) = mgr.on_shortcut("Alt+Shift+N", |app, _sc, event| {
        if event.state == ShortcutState::Pressed {
            if let Some(w) = app.get_webview_window("main") {
                if w.is_visible().unwrap_or(false) {
                    let _ = w.hide();
                } else {
                    let _ = w.unminimize();
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        }
    }) {
        db::debug_log(&format!(
            "全局快捷键 Alt+Shift+N 注册失败（可能被占用）: {e:?}"
        ));
    }
    // Alt+Shift+P：切换手动暂停（与托盘菜单同款：暂停前先结算应用使用时长）
    if let Err(e) = mgr.on_shortcut("Alt+Shift+P", |app, _sc, event| {
        if event.state == ShortcutState::Pressed {
            crate::toggle_pause(app);
        }
    }) {
        db::debug_log(&format!(
            "全局快捷键 Alt+Shift+P 注册失败（可能被占用）: {e:?}"
        ));
    }
}

#[tauri::command]
pub(crate) async fn refresh_holidays(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<u32, String> {
    let year = Local::now().year();
    match holiday::fetch_year(year).await {
        Ok(days) => {
            let c = holiday::HolidayCache {
                year,
                fetched_at: 0,
                days,
            };
            let mw = current_monthly_workdays(&sync::lock(&state.config, "state.config"), &c);
            crate::apply_holiday_cache(&app, c, true);
            Ok(mw)
        }
        Err(e) => {
            // 手动刷新失败也让数字先准起来：内置表已收录的年份直接兜底，
            // 未收录才把错误抛给前端（此时确实给不出可信的工作日数）。
            db::debug_log(&format!("手动刷新节假日失败: {}", e));
            match holiday::builtin_cache(year) {
                Some(c) => {
                    db::debug_log(&format!("节假日：改用内置 {year} 年法定节假日表"));
                    let mw =
                        current_monthly_workdays(&sync::lock(&state.config, "state.config"), &c);
                    crate::apply_holiday_cache(&app, c, false);
                    Ok(mw)
                }
                None => Err(format!(
                    "节假日数据获取失败，且内置表未收录 {year} 年：{}",
                    e
                )),
            }
        }
    }
}

#[tauri::command]
pub(crate) fn get_status_cmd(state: State<AppState>) -> calc::DayStatus {
    get_status(state.inner())
}

/// 隐藏主窗口（点关闭按钮时调用）
#[tauri::command]
pub(crate) fn hide_window(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

/// 显示主窗口：还原最小化 + 显示 + 抢焦点
/// （最小化状态下 show() 是无效操作，必须先 unminimize）
#[tauri::command]
pub(crate) fn show_window(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// 仅把主窗口提到前台
#[tauri::command]
pub(crate) fn focus_window(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// 前端调试日志落盘（写入 %APPDATA%/niuma-timer/debug.log，排查用户桌面环境用）
#[tauri::command]
pub(crate) fn write_debug_log(msg: String) {
    crate::db::debug_log(&msg);
}

/// 开机自启是否已开启。底层走 tauri-plugin-autostart（Windows 即 HKCU Run 键），
/// 用户在任务管理器里手工禁用后这里如实反映——注册表是唯一真相源。
#[tauri::command]
pub(crate) fn get_autostart(app: tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch()
        .is_enabled()
        .map_err(|e| format!("读取开机自启状态失败: {e:?}"))
}

/// 开启 / 关闭开机自启（经官方插件写 HKCU Run 键，无需管理员权限）。
/// 失败时把错误回给前端弹提示，避免开关显示成功、实际没写上。
#[tauri::command]
pub(crate) fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<String, String> {
    let mgr = app.autolaunch();
    if enabled {
        mgr.enable()
            .map(|_| "已开启开机自启".to_string())
            .map_err(|e| format!("开启开机自启失败: {e:?}"))
    } else {
        mgr.disable()
            .map(|_| "已关闭开机自启".to_string())
            .map_err(|e| format!("关闭开机自启失败: {e:?}"))
    }
}

/// 导出文件名清洗 + 落盘目录（export_csv / export_image 共用）：
/// 只取 basename、剔除 Windows 非法字符；目录回退链：下载 → 文档 → 临时目录。
fn safe_download_path(filename: &str) -> std::path::PathBuf {
    let base = std::path::Path::new(filename)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "export.csv".to_string());
    let safe: String = base
        .chars()
        .filter(|c| !"/\\:*?\"<>|".contains(*c))
        .collect();
    let safe = if safe.trim().is_empty() {
        "export.csv".to_string()
    } else {
        safe
    };
    let dir = dirs::download_dir()
        .or_else(dirs::document_dir)
        .unwrap_or_else(std::env::temp_dir);
    dir.join(&safe)
}

/// 导出 CSV：把 content（纯 UTF-8，不含 BOM）写到用户「下载」目录，返回最终保存路径。
/// 背景：早年以为 Tauri WebView 会取消 <a download>（实测并不取消，文件其实会落到
/// 下载目录）；真正的坑是纯前端下载没有任何可见反馈、用户会反复点。故统一走后端写盘，
/// 并由前端弹提示条告知落盘位置。
#[tauri::command]
pub(crate) fn export_csv(filename: String, content: String) -> Result<String, String> {
    let path = safe_download_path(&filename);
    // 写 UTF-8 + BOM，Excel 双击中文不乱码
    let mut bytes = b"\xef\xbb\xbf".to_vec();
    bytes.extend_from_slice(content.as_bytes());
    std::fs::write(&path, &bytes).map_err(|e| format!("写入失败：{}", e))?;
    Ok(path.to_string_lossy().into_owned())
}

/// 存图片（v1.6.0 周报图片）：前端 canvas 出 PNG 后 base64 传回落盘下载目录。
/// 下载被 WebView 取消的背景同 export_csv；base64 走 IPC 而非直接传二进制，
/// 与现有命令参数形态保持一致。
#[tauri::command]
pub(crate) fn export_image(filename: String, content_base64: String) -> Result<String, String> {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    let bytes = BASE64
        .decode(content_base64.trim())
        .map_err(|e| format!("图片数据解码失败: {e}"))?;
    if bytes.is_empty() {
        return Err("图片数据为空".to_string());
    }
    let path = safe_download_path(&filename);
    std::fs::write(&path, &bytes).map_err(|e| format!("写入失败：{}", e))?;
    Ok(path.to_string_lossy().into_owned())
}
