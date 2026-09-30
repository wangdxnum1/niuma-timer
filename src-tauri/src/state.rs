//! 共享应用状态与当日状态快照。
//!
//! 二期自 main.rs 切出（纯移动）：AppState 是全部模块的共享底座，
//! 字段 pub(crate)——main.rs 的接线函数（节假日缓存 / 跨天 / 加班检测）
//! 与各命令模块都要直接读写这些锁。

use std::sync::Mutex;

use chrono::{Datelike, Local, NaiveDate};

use crate::calc;
use crate::config;
use crate::holiday;
use crate::pause;
use crate::sync;

pub(crate) struct AppState {
    pub(crate) config: Mutex<config::Config>,
    pub(crate) holiday: Mutex<holiday::HolidayCache>,
    pub(crate) last_date: Mutex<NaiveDate>,
    /// 最近一次已处理的锁屏时间戳，用于检测新锁屏事件
    pub(crate) last_lock_seen: Mutex<Option<i64>>,
}

impl Default for AppState {
    fn default() -> Self {
        AppState {
            config: Mutex::new(config::load()),
            holiday: Mutex::new(holiday::HolidayCache::default()),
            last_date: Mutex::new(Local::now().date_naive()),
            last_lock_seen: Mutex::new(None),
        }
    }
}

/// 计算当月实际上班天数（手动覆盖 > 缓存 > 兜底周末数）
pub(crate) fn current_monthly_workdays(cfg: &config::Config, hol: &holiday::HolidayCache) -> u32 {
    let now = Local::now();
    if let Some(n) = config::effective_workdays_override(cfg, now.year(), now.month()) {
        return n;
    }
    if let Some(n) = hol.month_workdays(now.year(), now.month()) {
        return n;
    }
    holiday::weekday_count(now.year(), now.month())
}

/// 计算当天状态快照
pub(crate) fn get_status(state: &AppState) -> calc::DayStatus {
    let cfg = sync::lock(&state.config, "state.config").clone();
    let hol = sync::lock(&state.holiday, "state.holiday").clone();
    let now = Local::now();
    let is_workday = hol.is_workday(now.date_naive()).unwrap_or_else(|| {
        let wd = now.weekday().num_days_from_monday();
        wd < 5
    });
    let mw = current_monthly_workdays(&cfg, &hol);
    let mut st = calc::compute(&cfg, is_workday, mw, now);
    // v1.3.0 守护：暂停状态随每秒状态快照广播（托盘文案 / 前端徽章 / 悬停卡片共用）
    st.paused = pause::is_paused();
    st
}
