#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! 应用入口与接线层：插件装配、启动序列、托盘/调度器的业务钩子、命令注册。
//!
//! 二期模块化后，main.rs 只保留三类内容：入口与启动序列（main() + setup 闭包 +
//! run loop）；scheduler / tray 回调的接线函数（refresh_tray / 跨天 / 加班检测 /
//! 暂停切换）；各域命令模块的挂载（cmds_*：#[tauri::command] 定义见各模块，
//! 经 use 引入后 generate_handler 仍以裸名登记）。
//! 共享状态 AppState 与状态快照 get_status 在 state.rs，经根模块 re-export 供
//! remind / tray 以 `crate::` 路径引用（调用点零改动）。

mod activity;
mod app_usage;
mod audio_usage;
mod backup;
mod calc;
mod cmds_bill;
mod cmds_core;
mod cmds_debug;
mod cmds_monitor;
mod cmds_storage;
mod cmds_update;
mod config;
mod db;
mod diag;
mod holiday;
mod icon_render;
mod insights;
mod lock_monitor;
mod maintain;
mod overtime;
mod pause;
mod remind;
mod remote;
mod scheduler;
mod state;
mod sync;
mod tray;
mod update;
mod weekbill;
mod win;

use chrono::{Datelike, Local, TimeZone};
use tauri::Manager;
use tauri_plugin_autostart::MacosLauncher;

use crate::diag::{build_info, install_crash_log, show_fatal, trace_startup};
// 共享状态与状态快照的根模块 re-export：remind.rs / tray.rs 以 crate::AppState /
// crate::get_status 引用，scheduler.rs 调用的三个接线函数本就在本文件。
pub(crate) use crate::state::{get_status, AppState};

use cmds_bill::{
    delete_overtime_record, get_bill, get_body_bill, get_day_timeline, get_heatmap,
    get_overtime_records, get_trend, save_overtime_record,
};
use cmds_core::{
    apply_monitor_switches, apply_shortcuts, export_csv, focus_window, get_autostart,
    get_status_cmd, hide_window, load_config, refresh_holidays, save_config, set_autostart,
    show_window, write_debug_log,
};
use cmds_debug::{
    reset_remind_state, run_remind_tick, test_offwork_notify, test_sedentary_notify,
    test_sedentary_trigger,
};
use cmds_monitor::{get_activity_summary, get_app_usage_summary, get_audio_usage_summary};
use cmds_storage::{backup_now, get_storage_info, list_backups, restore_backup, run_maintenance};
use cmds_update::{check_update, skip_update_version, start_update, take_update_announcement};

/// 装载节假日数据并刷新托盘。
/// `persist=true` 表示数据来自网络，落本地缓存供下次启动直接用；
/// 内置兜底表不落盘——它只是「没有网络时的次优解」，不该污染缓存文件。
fn apply_holiday_cache(app: &tauri::AppHandle, mut c: holiday::HolidayCache, persist: bool) {
    if persist {
        c.fetched_at = Local::now().timestamp();
    }
    let state = app.state::<AppState>();
    {
        let mut hol = sync::lock(&state.holiday, "state.holiday");
        *hol = c;
        if persist {
            let snapshot = hol.clone();
            drop(hol);
            holiday::save_cache(&snapshot);
        }
    }
    let st = get_status(state.inner());
    tray::update_tray(app, &st);
}

/// 后台拉取并刷新节假日缓存；网络不可用时退回内置法定节假日表。
pub fn spawn_holiday_refresh(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let year = Local::now().year();
        match holiday::fetch_year(year).await {
            Ok(days) => {
                apply_holiday_cache(
                    &app,
                    holiday::HolidayCache {
                        year,
                        fetched_at: 0,
                        days,
                    },
                    true,
                );
            }
            Err(e) => {
                db::debug_log(&format!("节假日刷新失败: {}", e));
                match holiday::builtin_cache(year) {
                    Some(c) => {
                        db::debug_log(&format!(
                            "节假日：网络不可用，改用内置 {year} 年法定节假日表"
                        ));
                        apply_holiday_cache(&app, c, false);
                    }
                    None => db::debug_log(&format!(
                        "节假日：网络不可用且内置表未收录 {year} 年，退回周一至周五估算"
                    )),
                }
            }
        }
    });
}

/// 每秒刷新托盘实时状态（已赚¥ / 距下班 / 距发薪日）。仅 UI 刷新，无副作用，
/// 从主循环抽离以便独立调频率或单元测试。
fn refresh_tray(apph: &tauri::AppHandle, state: &AppState) {
    let st = get_status(state);
    tray::update_tray(apph, &st);
}

/// 跨天检测：日期变化（last_date 落后）时重拉当年节假日缓存。
/// 低频调用即可，无需秒级——托盘 UI 每秒仍按实时日期正常显示。
fn maybe_rollover_day(state: &AppState, apph: &tauri::AppHandle) {
    let today = Local::now().date_naive();
    let need_refresh = {
        let mut last = sync::lock(&state.last_date, "state.last_date");
        if *last != today {
            *last = today;
            true
        } else {
            false
        }
    };
    if need_refresh {
        spawn_holiday_refresh(apph.clone());
    }
}

/// 加班锁屏检测：last_lock_timestamp 变化且开启加班、且为工作日时，计算并落盘加班记录。
/// 原耦合在 1s 主循环里每秒轮询，现抽到低频业务线程，减少无谓的每秒锁竞争与计算。
fn maybe_record_overtime_lock(state: &AppState) {
    let cfg = sync::lock(&state.config, "state.config").clone();
    if !cfg.overtime_enabled {
        return;
    }
    let Some(lock_ts) = lock_monitor::last_lock_timestamp() else {
        return;
    };
    // 远程会话期间：锁屏事件不可信（连上/断开远程会伪造），跳过自动加班记录，
    // 但标记已处理，避免远程结束后又把这条错误时间补记上。
    if cfg.overtime_exclude_remote && crate::remote::is_remote_active(crate::remote::REMOTE_WINDOW)
    {
        crate::db::debug_log(&format!(
            "[overtime] 远程会话期间跳过自动加班记录（lock_ts={}）",
            lock_ts
        ));
        *sync::lock(&state.last_lock_seen, "state.last_lock_seen") = Some(lock_ts);
        return;
    }
    {
        let seen = sync::lock(&state.last_lock_seen, "state.last_lock_seen");
        if *seen == Some(lock_ts) {
            return;
        }
    }

    let hol = sync::lock(&state.holiday, "state.holiday");
    let persist = if let Some(lt) = Local.timestamp_opt(lock_ts, 0).single() {
        // 归属日由**锁屏时刻**自己决定，不能用「检测时刻」的日期：检测每 5 秒一次，
        // 23:59:58 锁屏可能在 00:00:02 才被处理，而凌晨锁屏要归属前一天。
        let (day, _, _) = overtime::resolve_overtime_day(lt);
        let kind = overtime::day_kind(day, Some(&hol));
        // 非工作日只在开关打开时才算加班。此前这里直接 `if !is_workday { return }`，
        // 导致 weekend_overtime 开关形同虚设——开了也永远走不到计算。
        if kind.is_rest() && !cfg.weekend_overtime {
            None
        } else {
            // 自动路径：只覆盖同为自动来源的记录，用户手改过的那天不会被顶掉
            overtime::calc_record_auto(lt, &cfg, Some(&hol)).map(overtime::upsert_auto)
        }
    } else {
        None
    };
    if overtime::should_mark_lock_seen(persist) {
        *sync::lock(&state.last_lock_seen, "state.last_lock_seen") = Some(lock_ts);
    }
}

/// 切换手动暂停（托盘菜单 / 全局快捷键 / 前端共用入口）。返回切换后的暂停状态。
///
/// 暂停前先结算一段应用使用时长（tick_now）——此刻 is_paused() 仍为 false，
/// 暂停前的最后一段会正常入账；再晚一步就会被暂停守卫丢掉。
pub fn toggle_pause(app: &tauri::AppHandle) -> bool {
    if pause::is_paused() {
        pause::set_manual(false);
    } else {
        app_usage::tick_now();
        pause::set_manual(true);
    }
    let state = app.state::<AppState>();
    let st = get_status(state.inner());
    tray::update_tray(app, &st);
    pause::is_paused()
}

/// 注入到 webview 的轻量 Tauri API 垫片。
/// 本版本未启用全局 window.__TAURI__，这里基于始终存在的
/// window.__TAURI_INTERNALS__.invoke 自行暴露 core.invoke 与 window 控制，
/// 省去前端打包 @tauri-apps/api 的步骤。
const INVOKE_SHIM: &str = r#"
if (!window.__TAURI__) {
  window.__TAURI__ = {
    core: {
      invoke: function (cmd, args) {
        return window.__TAURI_INTERNALS__.invoke(cmd, args || {});
      }
    },
    window: {
      getCurrentWindow: function () {
        return {
          hide: function () { return window.__TAURI_INTERNALS__.invoke('hide_window'); },
          show: function () { return window.__TAURI_INTERNALS__.invoke('show_window'); },
          setFocus: function () { return window.__TAURI_INTERNALS__.invoke('focus_window'); }
        };
      }
    },
    event: {
      // Tauri v2 的事件系统实际就是 plugin:event|listen 命令（见 @tauri-apps/api 的
      // _listen 实现），这里直接调它，省掉整个 @tauri-apps/api 依赖与打包链。
      // handler 收到的是完整 event 对象 { event, id, payload }。
      listen: function (evt, handler) {
        return window.__TAURI_INTERNALS__.invoke('plugin:event|listen', {
          event: evt,
          target: { kind: 'Any' },
          handler: window.__TAURI_INTERNALS__.transformCallback(handler)
        });
      }
    }
  };
}
"#;

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    if let Some(args) = update::parse_helper_args(&argv) {
        std::process::exit(update::run_helper(args));
    }
    if argv.iter().any(|arg| arg == "--apply-update") {
        std::process::exit(2);
    }
    install_crash_log();
    match update::resume_after_helper() {
        Ok(true) => return,
        Err(e) => {
            db::debug_log(&e);
            return;
        }
        Ok(false) => {}
    }
    // 两条日志都在 .setup 之前：webview 起不来时也能留下「装的是哪一版」的痕迹
    db::debug_log(&format!("build: {}", build_info()));
    trace_startup(&format!("build: {}", build_info()));
    let app = tauri::Builder::default()
        // 单例模式：若已有实例在运行，第二个实例启动时被拦截，
        // 并在回调里把已存在的主窗口显示并置前，自己退出。
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        // 系统通知（v1.3.0 守护）：主窗隐藏时久坐/下班提醒走系统通知通道
        .plugin(tauri_plugin_notification::init())
        // 全局快捷键（v1.3.0 守护）：Alt+Shift+N 显隐主窗 / Alt+Shift+P 切换暂停
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // 自动更新（v1.4.0 阶段 A）：前端不直接调插件命令，
        // 一律走 check_update / start_update 包装，故 capabilities 无需 updater:default
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .append_invoke_initialization_script(INVOKE_SHIM)
        .setup(|app| {
            trace_startup("setup: enter");
            // 单实例插件已取得进程所有权，数据库尚未打开；避免第二个实例误交换运行中的库。
            if let Some(msg) = backup::apply_pending_on_startup() {
                db::debug_log(&msg);
            }
            app.manage(AppState::default());
            // SQLite 首次调用 conn() 时建目录 + 开库 + 建表（WAL）。
            // 程序未发布、无旧库旧 JSON，不做任何迁移；schema 变更直接改 DDL 重建库。
            db::conn();
            trace_startup("setup: db ready");

            // 载入本地节假日缓存；无缓存时用内置法定节假日表兜底，
            // 避免首屏就按「周一至周五」估算（2026-10 会算成 22 天，实际 18 天）
            {
                let year = Local::now().year();
                if let Some(c) = holiday::load_cache(year).or_else(|| holiday::builtin_cache(year))
                {
                    *sync::lock(&app.state::<AppState>().holiday, "state.holiday") = c;
                } else {
                    db::debug_log(&format!(
                        "节假日：无缓存且内置表未收录 {year} 年，退回周一至周五估算"
                    ));
                }
            }
            trace_startup("setup: holiday cache loaded");
            // 修复任务栏图标模糊：用 exe 内嵌多尺寸 ico 按 DPI 重新设置窗口图标
            #[cfg(windows)]
            if let Some(w) = app.get_webview_window("main") {
                if let Ok(hwnd) = w.hwnd() {
                    crate::win::set_window_icons_from_resource(hwnd);
                }
            }
            trace_startup("setup: window icons set");

            // 绿色 exe 的系统通知通道：AUMID 未注册时 Windows 会静默丢弃 toast
            //（2026-09-23 排查：exe 放非 target 目录时插件误判已安装、用未注册 AUMID 发通知）。
            // WinRT toast 的 IconUri 指向 exe 提取图标不稳定，内嵌 ico 落盘后指向文件。
            // 每次启动幂等注册；失败只记日志，不影响主流程。
            #[cfg(windows)]
            {
                let icon_uri = match crate::win::write_aumid_icon_file(&config::config_dir()) {
                    Ok(p) => p.to_string_lossy().to_string(),
                    Err(e) => {
                        db::debug_log(&format!("AUMID 图标写出失败，退回 exe 图标: {e}"));
                        std::env::current_exe()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default()
                    }
                };
                let icon = if icon_uri.is_empty() {
                    None
                } else {
                    Some(icon_uri.as_str())
                };
                if let Err(e) =
                    crate::win::register_aumid(&app.config().identifier, "牛马计时器", icon)
                {
                    db::debug_log(&format!("AUMID 注册失败，系统通知将不弹: {e}"));
                }
            }

            // 创建托盘
            let _tray = tray::create_tray(app)?;
            trace_startup("setup: tray created");

            // 启动页防白闪：窗口初始 visible:false，由前端 splash 渲染完成后 show；
            // 此处兜底——1 秒后无论如何 show，避免前端 JS 异常导致窗口永久不可见
            {
                let app2 = app.handle().clone();
                let _ = std::thread::Builder::new()
                    .name("niuma-startup-show".to_string())
                    .spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(1000));
                        if let Some(w) = app2.get_webview_window("main") {
                            let _ = w.show();
                        }
                    });
            }

            // 启动锁屏监听线程（Windows session notification）
            lock_monitor::start();
            trace_startup("setup: lock monitor started");

            // 按配置初始化三个监控开关。活动监控关闭时会真正注销 Raw Input 设备
            // （而非回调里空转），故必须在 activity::start() 之前调用。
            apply_monitor_switches(
                &sync::lock(&app.state::<AppState>().config, "state.config").clone(),
            );
            trace_startup("setup: monitor switches applied");

            // 注册全局快捷键（Alt+Shift+N / Alt+Shift+P，可被配置关闭）
            apply_shortcuts(
                app.handle(),
                &sync::lock(&app.state::<AppState>().config, "state.config").clone(),
            );
            trace_startup("setup: shortcuts applied");

            // 启动鼠标/键盘活动统计（Raw Input 旁路采集，常驻托盘即持续统计，输入法零延迟）
            activity::start();
            trace_startup("setup: activity started");

            // 启动应用使用时长监控（前台窗口事件钩子，统计微信等白名单应用）
            app_usage::start();
            trace_startup("setup: app_usage started");

            // 启动媒体播放时长监控（音频会话峰值轮询，统计正在发声的软件）
            audio_usage::start();
            trace_startup("setup: audio_usage started");

            // 窗口关闭仅隐藏，不退出程序
            if let Some(w) = app.get_webview_window("main") {
                w.clone().on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = w.hide();
                    }
                });
            }

            let apph = app.handle().clone();
            // 启动即拉一次节假日
            spawn_holiday_refresh(apph.clone());

            // 统一周期调度：托盘 UI 刷新(1s) / 跨天+加班检测(5s) / 活动落盘与
            // 应用结算(10s) 合并成一条「1 秒一拍」的线程——它们都是纯周期任务，
            // 原先各占一个常驻线程纯属浪费。带 Win32 消息循环 / COM 亲和的采集
            // 线程不在此列，仍在各自模块里（详见 scheduler 模块文档）。
            // 必须在 activity::start() / app_usage::start() 之后：采集线程先就位。
            scheduler::start(apph);
            trace_startup("setup: scheduler started");

            // 延迟 30 秒自动检查一次更新，之后每 6 小时一次（设置页可关）
            update::spawn_update_checker(app.handle().clone());
            trace_startup("setup: update checker started");

            trace_startup("setup: done");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_config,
            save_config,
            refresh_holidays,
            get_status_cmd,
            test_offwork_notify,
            test_sedentary_notify,
            test_sedentary_trigger,
            reset_remind_state,
            run_remind_tick,
            hide_window,
            show_window,
            focus_window,
            get_overtime_records,
            save_overtime_record,
            delete_overtime_record,
            get_activity_summary,
            get_app_usage_summary,
            get_audio_usage_summary,
            get_bill,
            get_heatmap,
            get_trend,
            get_body_bill,
            get_day_timeline,
            write_debug_log,
            get_autostart,
            set_autostart,
            get_storage_info,
            run_maintenance,
            export_csv,
            check_update,
            start_update,
            skip_update_version,
            take_update_announcement,
            backup_now,
            list_backups,
            restore_backup
        ])
        .build(tauri::generate_context!());
    match app {
        Ok(app) => {
            trace_startup("app: built, entering run loop");
            app.run(|_app_handle, event| {
                // 程序退出前：把鼠标/键盘统计与应用使用时长的最后增量落盘，重启后不丢数据
                if let tauri::RunEvent::Exit = event {
                    activity::shutdown();
                    app_usage::shutdown();
                    audio_usage::shutdown();
                }
            });
        }
        Err(e) => {
            // build 失败（最常见：目标机缺 WebView2 运行时）→ 弹窗告知具体原因，
            // 不再静默退出让用户误以为「双击无反应」。
            let detail = e.to_string();
            let hint = if detail.to_lowercase().contains("webview") {
                "\n\n推测原因：系统缺少 WebView2 运行时（Win11 通常自带；干净/企业精简镜像可能未预装）。\n请到微软官网下载安装「WebView2 Runtime (Evergreen Bootstrapper)」后重试。"
            } else {
                ""
            };
            let msg = format!("牛马计时器启动失败：\n{detail}{hint}");
            show_fatal(&msg);
            std::process::exit(1);
        }
    }
    // 前端资源自动重编：build.rs 用 frontend 目录内容指纹注入
    // `cargo:rustc-env=TAURI_FRONTEND_FP`，内容一变即触发本 crate 重编、
    // generate_context! 重跑并重新嵌入最新 HTML/CSS/JS，无需手动改动任何标记。
}
