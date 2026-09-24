//! 提醒（v1.3.0「牛马守护」）：久坐提醒 + 下班提醒。
//!
//! 调度：由 scheduler 每 60 秒调用一次 [`tick`]（分钟级阈值无需秒级精度）。
//! 投递：统一走 tauri-plugin-notification 系统通知（[`notify`]），
//! 主窗可见与否不影响通道（v1.3.1 起移除横幅双通道）。
//!
//! 久坐口径：
//! - 「在活跃」= 最近一次键鼠输入距今 < 5 分钟（纯挂机不键入不算连续活跃）；
//! - 暂停（手动）与锁屏期间不计时，连续起点清零；
//! - 连续活跃 ≥ 阈值分钟 → 触发；触发后清零重新起算，且 5 分钟冷却防轰炸。
//!
//! 下班口径：工作日 + 已过 `pm_end` + 当日有监控记录 + 今天未提醒过 → 触发一次。
//! 程序在过点后才启动也能补弹（today_done 初始为 false，首拍即满足）。

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::Timelike;
use tauri::Manager;
use tauri_plugin_notification::NotificationExt;

use crate::config::Config;
use crate::sync;

/// 键鼠 idle 超过此时长视为「人已离开/在休息」，连续活跃计时清零
const IDLE_REST_MS: i64 = 5 * 60 * 1000;
/// 久坐提醒冷却：触发后该时长内不重复提醒
const REMIND_COOLDOWN_MS: i64 = 5 * 60 * 1000;

/// 连续活跃起点（毫秒墙钟，0 = 未在计时 / 上一拍判定为离开）
static SED_SINCE: Mutex<i64> = Mutex::new(0);
/// 上次久坐提醒时刻（毫秒墙钟，0 = 从未提醒）
static LAST_SED_REMIND: Mutex<i64> = Mutex::new(0);
/// 下班提醒当天是否已触发
static OFFWORK_DONE: AtomicBool = AtomicBool::new(false);
/// OFFWORK_DONE 所属日期（跨天自动重置，自包含、无需调度器配合）
static OFFWORK_DATE: Mutex<Option<chrono::NaiveDate>> = Mutex::new(None);

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 久坐提醒判定（纯函数，全参数注入，可离线单测）。
/// 返回 true = 本拍应触发提醒。
#[allow(clippy::too_many_arguments)]
pub fn should_remind(
    now_ms: i64,
    active_since_ms: i64,
    last_input_ms: u64,
    paused: bool,
    away: bool,
    threshold_min: i64,
    last_remind_ms: i64,
) -> bool {
    // 暂停 / 锁屏：不计时也不提醒（active_since 已被调用方清零，双保险）
    if paused || away || active_since_ms <= 0 {
        return false;
    }
    // 挂机满 5 分钟 = 已经休息过了，连续计时应被打断
    if now_ms.saturating_sub(last_input_ms as i64) >= IDLE_REST_MS {
        return false;
    }
    let threshold = threshold_min.max(1) * 60_000;
    if now_ms.saturating_sub(active_since_ms) < threshold {
        return false;
    }
    // 冷却期内不重复轰炸；从未提醒过（<=0）不受冷却限制
    last_remind_ms <= 0 || now_ms.saturating_sub(last_remind_ms) >= REMIND_COOLDOWN_MS
}

/// 下班提醒判定（纯函数，可离线单测）。
/// 「补弹」：过点后才启动程序时 today_done=false，首拍即满足全部条件——无需特殊分支。
pub fn should_remind_offwork(
    today_done: bool,
    is_workday: bool,
    now_min: f64,
    pm_end_min: f64,
    has_record_today: bool,
) -> bool {
    !today_done && is_workday && now_min >= pm_end_min && has_record_today
}

/// 60 秒一拍的提醒调度入口（scheduler 调用）
pub fn tick(app: &tauri::AppHandle) {
    let cfg = {
        let state = app.state::<crate::AppState>();
        let cfg = sync::lock(&state.config, "state.config").clone();
        cfg
    };
    sedentary_tick(app, &cfg);
    offwork_tick(app, &cfg);
}

fn sedentary_tick(app: &tauri::AppHandle, cfg: &Config) {
    if !cfg.remind_sedentary_enabled {
        *sync::lock(&SED_SINCE, "remind::SED_SINCE") = 0;
        return;
    }
    let paused = crate::pause::is_paused();
    let away = crate::lock_monitor::is_away();
    let now = now_ms();
    let last_input = crate::activity::last_input_ms();
    // 状态推进：离开（暂停 / 锁屏 / 挂机满 5 分钟）→ 清零；活跃且无起点 → 重新起算
    {
        let mut g = sync::lock(&SED_SINCE, "remind::SED_SINCE");
        let idle = now.saturating_sub(last_input as i64);
        if paused || away || idle >= IDLE_REST_MS {
            *g = 0;
        } else if *g == 0 {
            *g = now;
        }
    }
    let since = *sync::lock(&SED_SINCE, "remind::SED_SINCE");
    let last = *sync::lock(&LAST_SED_REMIND, "remind::LAST_SED_REMIND");
    let minutes = cfg.remind_sedentary_minutes as i64;
    if should_remind(now, since, last_input, paused, away, minutes, last) {
        // 触发：先重置状态再投递（投递失败也不至于连环轰炸）
        *sync::lock(&SED_SINCE, "remind::SED_SINCE") = 0;
        *sync::lock(&LAST_SED_REMIND, "remind::LAST_SED_REMIND") = now;
        let (title, body) = sedentary_texts(minutes);
        notify(app, &title, &body);
    }
}

fn offwork_tick(app: &tauri::AppHandle, cfg: &Config) {
    let today = chrono::Local::now().date_naive();
    // 跨天重置：昨天提醒过，今天要重新提醒
    {
        let mut d = sync::lock(&OFFWORK_DATE, "remind::OFFWORK_DATE");
        if *d != Some(today) {
            *d = Some(today);
            OFFWORK_DONE.store(false, Ordering::SeqCst);
        }
    }
    if OFFWORK_DONE.load(Ordering::Relaxed) || !cfg.remind_offwork_enabled {
        return;
    }
    let st = crate::get_status(app.state::<crate::AppState>().inner());
    let now = chrono::Local::now();
    let now_min = now.hour() as f64 * 60.0 + now.minute() as f64 + now.second() as f64 / 60.0;
    if !should_remind_offwork(
        OFFWORK_DONE.load(Ordering::Relaxed),
        st.is_workday,
        now_min,
        crate::calc::to_min(&cfg.pm_end),
        has_record_today(),
    ) {
        return;
    }
    OFFWORK_DONE.store(true, Ordering::SeqCst);
    let (title, body) = offwork_texts(st.earned);
    notify(app, &title, &body);
}

/// 当日是否有监控记录（键鼠活动 或 应用使用任一非空）。
/// 没记录 = 今天根本没在工位，提醒「下班」没有意义。
fn has_record_today() -> bool {
    let today = chrono::Local::now().date_naive().format("%Y-%m-%d").to_string();
    crate::db::with_db(|g| {
        let n: i64 = g.query_row(
            "SELECT (SELECT COUNT(*) FROM act_hourly WHERE date = ?1) \
                  + (SELECT COUNT(*) FROM app_usage WHERE date = ?1)",
            [&today],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    })
    .unwrap_or(false)
}

/// 系统通知投递（tauri-plugin-notification）。允许失败——提醒不该影响主流程，
/// 但失败必须留痕：2026-09-23 曾因 AUMID 未注册被系统静默丢弃，排查零线索。
pub(crate) fn notify(app: &tauri::AppHandle, title: &str, body: &str) {
    if let Err(e) = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show()
    {
        crate::db::debug_log(&format!("通知投递失败 [{title}]: {e:?}"));
    }
}

/// 久坐提醒文案（真实触发与调试卡「休息提醒」按钮共用，避免两处文案漂移）
pub(crate) fn sedentary_texts(minutes: i64) -> (String, String) {
    (
        "该起来活动了".to_string(),
        format!("已连续搬砖 {minutes} 分钟，起来喝口水 🐎"),
    )
}

/// 下班提醒文案（真实触发与调试卡「下班提醒」按钮共用，同上）
pub(crate) fn offwork_texts(earned: f64) -> (String, String) {
    (
        "到点了，下班吧牛马".to_string(),
        format!("今天已赚 ¥{earned:.2}，别卷了 🐎"),
    )
}

/// 清空提醒运行时状态：下班「今天已提醒」标记 + 久坐冷却与连续计时。
/// 供调试卡「重置提醒状态」调用——下班提醒每天只触发一次，不重置当天就没法
/// 复测真实触发链路。
pub(crate) fn reset_state() {
    OFFWORK_DONE.store(false, Ordering::Relaxed);
    *sync::lock(&OFFWORK_DATE, "remind::OFFWORK_DATE") = None;
    *sync::lock(&LAST_SED_REMIND, "remind::LAST_SED_REMIND") = 0;
    *sync::lock(&SED_SINCE, "remind::SED_SINCE") = 0;
}

/// 调试卡「模拟久坐」：把连续活跃起点前拨 `minutes` 分钟并清掉冷却，
/// 让紧接着的一拍 tick 立刻满足「连续活跃 ≥ 阈值」。
///
/// 只动「起点」这一项运行时状态，判定逻辑 [`should_remind`] 零特判——触发走的是真实
/// 链路（判定 / 文案 / 通知 / 状态重置），与真实久坐行为一致，只是不必干等阈值分钟数。
/// 仍需满足「最近 5 分钟内有键鼠输入」（点按钮本身即满足），否则会被判为已休息。
pub(crate) fn force_sedentary_since(minutes: i64) {
    let back = minutes.max(1) * 60_000;
    *sync::lock(&SED_SINCE, "remind::SED_SINCE") = now_ms() - back;
    *sync::lock(&LAST_SED_REMIND, "remind::LAST_SED_REMIND") = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: i64 = 60_000; // 一分钟（毫秒）

    #[test]
    fn sedentary_requires_active_state() {
        // 暂停 / 锁屏 / 无起点 → 不提醒
        assert!(!should_remind(100 * M, 40 * M, 99 * M as u64, true, false, 50, 0));
        assert!(!should_remind(100 * M, 40 * M, 99 * M as u64, false, true, 50, 0));
        assert!(!should_remind(100 * M, 0, 99 * M as u64, false, false, 50, 0));
    }

    #[test]
    fn sedentary_idle_five_minutes_resets() {
        // 挂机满 5 分钟：视为已休息，不提醒（即便连续起点早已超过阈值）
        let idle = IDLE_REST_MS;
        assert!(!should_remind(100 * M, 40 * M, (100 * M - idle) as u64, false, false, 50, 0));
        // 刚好差 1ms 恢复活跃：正常判定
        assert!(should_remind(100 * M, 40 * M, (100 * M - idle + 1) as u64, false, false, 50, 0));
    }

    #[test]
    fn sedentary_threshold_boundary() {
        // 阈值边界：恰好达到 → 提醒；差 1ms → 不提醒
        let since = 50 * M;
        let now = since + 50 * M;
        assert!(should_remind(now, since, (now - 1) as u64, false, false, 50, 0));
        assert!(!should_remind(now - 1, since, (now - 2) as u64, false, false, 50, 0));
    }

    #[test]
    fn sedentary_cooldown_suppresses_repeat() {
        // 冷却期内不重复；冷却已过 / 从未提醒过 → 放行
        let since = 50 * M;
        let now = since + 50 * M;
        assert!(!should_remind(now, since, (now - 1) as u64, false, false, 50, now - 4 * M));
        assert!(should_remind(now, since, (now - 1) as u64, false, false, 50, now - 5 * M));
        assert!(should_remind(now, since, (now - 1) as u64, false, false, 50, 0));
    }

    #[test]
    fn offwork_conditions() {
        // 全条件满足（含补弹：过点后启动首拍即触发）
        assert!(should_remind_offwork(false, true, 18.0 * 60.0, 18.0 * 60.0, true));
        // 非工作日 / 未过点 / 已提醒 / 空记录日 → 一律不提醒
        assert!(!should_remind_offwork(false, false, 19.0 * 60.0, 18.0 * 60.0, true));
        assert!(!should_remind_offwork(false, true, 17.0 * 60.0 + 59.0, 18.0 * 60.0, true));
        assert!(!should_remind_offwork(true, true, 19.0 * 60.0, 18.0 * 60.0, true));
        assert!(!should_remind_offwork(false, true, 19.0 * 60.0, 18.0 * 60.0, false));
    }

    #[test]
    fn notify_texts_shared_by_real_and_test_paths() {
        let (title, body) = offwork_texts(123.456);
        assert_eq!(title, "到点了，下班吧牛马");
        assert!(body.contains("¥123.46"), "金额应保留两位小数，实际: {body}");

        let (title, body) = sedentary_texts(50);
        assert_eq!(title, "该起来活动了");
        assert!(body.contains("50 分钟"), "分钟数应出现在文案，实际: {body}");
    }

    #[test]
    fn reset_state_clears_all_remind_marks() {
        // 预置「今天已提醒过」的状态，reset 后应全部回到初始值
        OFFWORK_DONE.store(true, Ordering::Relaxed);
        *sync::lock(&OFFWORK_DATE, "test::OFFWORK_DATE") = Some(chrono::Local::now().date_naive());
        *sync::lock(&LAST_SED_REMIND, "test::LAST_SED_REMIND") = 12345;
        *sync::lock(&SED_SINCE, "test::SED_SINCE") = 67890;

        reset_state();

        assert!(!OFFWORK_DONE.load(Ordering::Relaxed), "下班提醒标记应清零");
        assert!(
            sync::lock(&OFFWORK_DATE, "test::OFFWORK_DATE").is_none(),
            "日期标记应清空"
        );
        assert_eq!(
            *sync::lock(&LAST_SED_REMIND, "test::LAST_SED_REMIND"),
            0,
            "久坐冷却应清零"
        );
        assert_eq!(*sync::lock(&SED_SINCE, "test::SED_SINCE"), 0, "连续活跃起点应清零");
    }
}
