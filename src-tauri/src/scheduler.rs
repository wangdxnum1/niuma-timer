//! 统一周期调度：所有低频周期任务共用一个「1 秒一拍」的线程。
//!
//! 背景（痛点 P1 线程治理）：此前「托盘 UI 刷新（1s）」「跨天 + 加班检测（5s）」
//! 「活动统计落盘（10s）」「应用使用结算（10s）」各起一个常驻线程，四个独立
//! sleep 循环占掉本程序常驻线程的一大半，而它们都是纯周期任务，没有任何理由
//! 各占一个线程——真正**必须**独占线程的不在此列，也不受本次改动影响：
//!
//! - `activity::raw_thread` / `app_usage::watch_thread` / `lock_monitor`：
//!   都要跑 Win32 消息循环，线程一旦被别的任务占住，消息就收不到了；
//! - `audio_usage::tick_loop`：COM 初始化与峰值采样必须固定在同一线程；
//! - `tray` 悬停卡片 actor：靠自身超时驱动，有独立的唤醒节奏。
//!
//! 这里按「节拍计数取模」分派：单拍该跑哪些任务由周期决定。
//! 每个任务都套 `catch_unwind`——某个任务 panic 只丢这一拍，不会让其余周期
//! 任务跟着一起停摆（panic 详情仍由 main.rs 的 panic hook 写进 panic.log）。

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

/// 基础节拍
const TICK: Duration = Duration::from_secs(1);
/// 跨天检测 + 加班锁屏落盘的周期（单位：拍）
const EVERY_5S: u64 = 5;
/// 活动统计落盘 + 应用使用结算的周期（单位：拍）
const EVERY_10S: u64 = 10;
/// 判断「是否到了新的一天」的检查周期（单位：拍）。每分钟看一次足够，
/// 不必每拍都取系统日期。
const EVERY_60S: u64 = 60;

/// 启动调度线程。幂等，仅首次生效。
///
/// 应在 `activity::start()` / `app_usage::start()` 之后调用：本线程只负责周期性
/// 落盘与结算，采集线程先就位才不会漏掉第一拍之前的数据。
pub fn start(app: AppHandle) {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    thread::spawn(move || {
        let mut beats: u64 = 0;
        let mut last_visible: Option<bool> = None;
        let mut last_maint: Option<chrono::NaiveDate> = None;
        loop {
            thread::sleep(TICK);
            beats = beats.wrapping_add(1);

            // ── 每拍（1s）：主窗口可见性广播 + 托盘实时状态 ──
            //
            // 可见性放在状态侧统一检测，而不是逐个改 show/hide 调用点：触发路径
            // 太多（托盘菜单 / hide、show 命令 / 关闭按钮拦截 / 前端 Esc），只有
            // 统一检测才能全覆盖。前端据此暂停 2 秒轮询省电；窗口隐藏时这段照跑，
            // 代价仅一次 bool 比较。
            let vis = app
                .get_webview_window("main")
                .map(|w| w.is_visible().unwrap_or(false))
                .unwrap_or(false);
            if last_visible != Some(vis) {
                last_visible = Some(vis);
                let _ = app.emit("win-visibility", vis);
            }
            {
                let state = app.state::<crate::AppState>();
                run("refresh_tray", || crate::refresh_tray(&app, state.inner()));
            }

            // ── 每 5 拍：跨天重拉节假日 + 锁屏加班落盘 ──
            // 从 1s 放宽到 5s 对加班统计无影响（锁屏→记加班的延迟 ≤5s）。
            if beats % EVERY_5S == 0 {
                let state = app.state::<crate::AppState>();
                run("rollover_day", || {
                    crate::maybe_rollover_day(state.inner(), &app)
                });
                run("overtime_lock", || {
                    crate::maybe_record_overtime_lock(state.inner())
                });
            }

            // ── 每 10 拍：活动统计落盘 + 应用使用结算 ──
            if beats % EVERY_10S == 0 {
                run("activity_flush", crate::activity::flush_now);
                run("app_usage_tick", crate::app_usage::tick_now);
            }

            // ── 每日一次：数据生命周期维护（WAL 收缩 + 过期图标 / 数据清理）──
            // 用「日期变了」而不是纯节拍计数：程序重启后当天还能补跑一次，
            // 不会因计数归零而漏掉一整天的维护（WAL 会一直胖着）。
            if beats % EVERY_60S == 0 {
                let today = chrono::Local::now().date_naive();
                if last_maint != Some(today) {
                    last_maint = Some(today);
                    let app2 = app.clone();
                    run("maintenance", move || {
                        let state = app2.state::<crate::AppState>();
                        let cfg = crate::sync::lock(&state.config, "state.config").clone();
                        crate::maintain::run_daily(&cfg);
                    });
                }
            }
        }
    });
}

/// 跑一个周期任务，兜住 panic：调度线程必须活下去，单个任务的失败只损失一拍。
fn run(name: &str, f: impl FnOnce()) {
    if catch_unwind(AssertUnwindSafe(f)).is_err() {
        crate::db::debug_log(&format!("[scheduler] 任务 {name} 本轮 panic，已跳过"));
    }
}
