//! 专注段统计（v1.7.0）：工作类前台 + 键鼠活跃连续达标即记一段。
//!
//! 设计要点：
//! - **纯状态机 + 薄包装**：判定逻辑全部在 [`tick_run`]（显式输入、不碰全局），
//!   单测直接驱动；全局包装只负责取信号（系统 idle / 前台应用 / 暂停 / 远程）
//!   和落库；
//! - **段结束才落库一条**：现有 act/app_usage 全是小时桶，粒度撑不起「连续
//!   25 分钟」判定，逐分钟建表又无谓——状态机在内存里跑，断段时一次性写入
//!   `focus_sessions`（每天几条到十几条）；
//! - **诚实口径**：段时长按「最后一次确认活跃的时刻」结算，系统睡眠/挂机的
//!   空窗不计入；不足阈值直接丢弃（「专注段」定义本身即达标线）；
//! - 断段条件：切到非工作类前台（摸鱼/沟通/其他都算分心）、无输入 ≥5 分钟
//!   （与守护提醒同一 idle 口径）、手动暂停（钱冻结，专注也冻结）、远程会话
//!   （输入来自远程设备不可信，与加班排除共用判定）、功能关闭。

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{Datelike, Local, NaiveDate, TimeZone, Timelike};
use serde::Serialize;

use crate::app_usage::{self, CAT_WORK};
use crate::config::Config;
use crate::db;
use crate::holiday::HolidayCache;
use crate::pause;
use crate::remote;
use crate::sync;
use crate::weekbill::{is_workday_of, period_bounds, period_label_of, Span};

/// 无输入多久视为离开（毫秒）。与守护提醒的 idle 口径一致。
const IDLE_GAP_MS: u64 = 5 * 60 * 1000;

use crate::weekbill::weekday_cn;

fn now_ms() -> i64 {
    Local::now().timestamp_millis()
}

fn hm_of(ms: i64) -> String {
    match Local.timestamp_millis_opt(ms).single() {
        Some(t) => format!("{:02}:{:02}", t.hour(), t.minute()),
        None => String::from("--:--"),
    }
}

// ---------- 纯状态机（单测驱动，不碰全局） ----------

/// 一次 tick 的显式输入
pub(crate) struct TickInput {
    /// 墙钟毫秒（段时间戳用墙钟：要落成「几点到几点」给人看）
    pub now_ms: i64,
    /// 最近 5 分钟内有键鼠输入
    pub active: bool,
    /// 当前前台是工作类应用时，为其展示名
    pub work_app: Option<String>,
}

/// 断段时产生的落库记录（不足阈值时为 None，直接丢弃）
pub(crate) struct ClosedSession {
    pub start_ms: i64,
    pub end_ms: i64,
    pub minutes: i64,
    pub top_app: String,
}

/// 进行中的一段
pub(crate) struct RunState {
    start_ms: i64,
    last_active_ms: i64,
    /// 各工作类应用的在段 tick 计数（top_app 依据）
    apps: HashMap<String, i64>,
}

/// 处理一拍：`run = None` 表示当前没有进行中的段。
/// 工作+活跃 → 起段/延长；任一不满足 → 断段（达标则产出落库记录）。
pub(crate) fn tick_run(
    run: &mut Option<RunState>,
    input: TickInput,
    threshold_min: i64,
) -> Option<ClosedSession> {
    let working = input.active && input.work_app.is_some();
    match (working, run.take()) {
        (true, None) => {
            // 起段：本拍只建基准
            let mut apps = HashMap::new();
            if let Some(a) = input.work_app {
                apps.insert(a, 1);
            }
            *run = Some(RunState {
                start_ms: input.now_ms,
                last_active_ms: input.now_ms,
                apps,
            });
            None
        }
        (true, Some(mut r)) => {
            r.last_active_ms = input.now_ms;
            if let Some(a) = input.work_app {
                *r.apps.entry(a).or_insert(0) += 1;
            }
            *run = Some(r);
            None
        }
        (false, Some(r)) => close_session(r, threshold_min),
        (false, None) => None,
    }
}

/// 结算一段：时长按「最后一次确认活跃」计（睡眠/挂机空窗不计入），
/// 达标产出落库记录，不足阈值丢弃。
fn close_session(run: RunState, threshold_min: i64) -> Option<ClosedSession> {
    let elapsed_ms = run.last_active_ms - run.start_ms;
    if elapsed_ms >= threshold_min * 60_000 {
        let top_app = run
            .apps
            .into_iter()
            .max_by_key(|(_, c)| *c)
            .map(|(k, _)| k)
            .unwrap_or_default();
        Some(ClosedSession {
            start_ms: run.start_ms,
            end_ms: run.last_active_ms,
            minutes: (elapsed_ms as f64 / 60_000.0).round() as i64,
            top_app,
        })
    } else {
        None
    }
}

// ---------- 全局包装（scheduler 每秒调用） ----------

static RUN: Mutex<Option<RunState>> = Mutex::new(None);

/// 每秒一拍：取信号 → 状态机 → 断段落库。
/// cfg/暂停/远程/系统 idle 全部在这里收敛成状态机的显式输入。
pub fn tick(cfg: &Config) {
    let input = if !cfg.focus_enabled
        || pause::is_paused()
        || remote::is_remote_active(remote::REMOTE_WINDOW)
    {
        // 关闭 / 暂停 / 远程：一律视为「不满足条件」（已有段则断段结算）
        TickInput {
            now_ms: now_ms(),
            active: false,
            work_app: None,
        }
    } else {
        // 系统级 idle：查不到时保守视为离开（宁可少记不虚记）
        let idle = crate::win::idle_ms().unwrap_or(IDLE_GAP_MS);
        let active = idle < IDLE_GAP_MS;
        let work_app = if active {
            app_usage::current_display().filter(|d| app_usage::category_of(d, cfg) == CAT_WORK)
        } else {
            None
        };
        TickInput {
            now_ms: now_ms(),
            active,
            work_app,
        }
    };
    let threshold = cfg.focus_min_minutes.clamp(10, 120) as i64;
    let closed = tick_run(&mut sync::lock(&RUN, "focus::RUN"), input, threshold);
    if let Some(s) = closed {
        insert_session(&s);
    }
}

/// 退出前结算进行中的专注段（ main.rs 的 RunEvent::Exit 调用）。
///
/// 段只在「闭合」时落库（见本模块开头），而托盘工具是按工作日起停的：
/// 不在这里收口，退出瞬间正在进行的那一段——
/// 往往正是当天最长的一段——会被静默丢弃，且不留任何痕迹。
/// 暂停 / 远程 / 关闭功能都已由 tick 收敛成断段，唯独退出漏了。
pub fn shutdown(cfg: &Config) {
    let input = TickInput {
        now_ms: now_ms(),
        active: false,
        work_app: None,
    };
    let threshold = cfg.focus_min_minutes.clamp(10, 120) as i64;
    if let Some(s) = tick_run(&mut sync::lock(&RUN, "focus::RUN"), input, threshold) {
        insert_session(&s);
    }
}

fn insert_session(s: &ClosedSession) {
    let date = Local
        .timestamp_millis_opt(s.start_ms)
        .single()
        .map(|t| t.date_naive().format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    let start_hm = hm_of(s.start_ms);
    let end_hm = hm_of(s.end_ms);
    let r = db::with_db(|conn| {
        conn.execute(
            "INSERT INTO focus_sessions (date, start_hm, end_hm, minutes, top_app) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![date, start_hm, end_hm, s.minutes, s.top_app],
        )
    });
    if let Err(e) = r {
        // 专注数据非关键路径：落库失败只记日志，绝不影响主流程
        db::debug_log(&format!("[focus] 专注段落库失败: {e}"));
    }
}

// ---------- 周期聚合（洞察「专注」tab） ----------

#[derive(Debug, Clone, Serialize)]
pub struct FocusDay {
    pub date: String,
    pub weekday: String,
    pub is_workday: bool,
    pub sessions: i64,
    pub total_min: i64,
    pub longest_min: i64,
    /// 最长一段的「开始–结束」（跨午夜显示 "23:50–次日 00:40"）
    pub longest_hm: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FocusSummary {
    pub period_start: String,
    pub period_end: String,
    pub period_label: String,
    /// 只含有数据的日期（按日期升序）
    pub days: Vec<FocusDay>,
    pub total_sessions: i64,
    pub total_min: i64,
    pub longest_min: i64,
    /// 最专注一天
    pub best_day: Option<FocusDay>,
}

/// 聚合内核（conn 显式传入，单测用 in-memory 库）
pub(crate) fn summary_assemble(
    start: NaiveDate,
    end: NaiveDate,
    span: Span,
    hol: &HolidayCache,
    conn: &rusqlite::Connection,
) -> rusqlite::Result<FocusSummary> {
    let start_s = start.format("%Y-%m-%d").to_string();
    let end_s = end.format("%Y-%m-%d").to_string();
    let mut by_date: HashMap<String, (i64, i64, i64, Option<String>)> = HashMap::new();
    {
        let mut st = conn.prepare(
            "SELECT date, start_hm, end_hm, minutes FROM focus_sessions \
             WHERE date >= ?1 AND date <= ?2 ORDER BY date, start_hm",
        )?;
        let rows = st
            .query_map(rusqlite::params![start_s, end_s], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (date, sh, eh, minutes) in rows {
            let e = by_date.entry(date).or_insert((0, 0, 0, None));
            e.0 += 1;
            e.1 += minutes;
            if minutes > e.2 {
                let hm = if eh < sh {
                    format!("{sh}–次日 {eh}")
                } else {
                    format!("{sh}–{eh}")
                };
                e.2 = minutes;
                e.3 = Some(hm);
            }
        }
    }

    let mut days: Vec<FocusDay> = by_date
        .into_iter()
        .map(|(date, (sessions, total_min, longest_min, longest_hm))| {
            let d = NaiveDate::parse_from_str(&date, "%Y-%m-%d");
            FocusDay {
                weekday: d.as_ref().map(|d| weekday_cn(*d)).unwrap_or("").to_string(),
                is_workday: d.map(|d| is_workday_of(d, hol)).unwrap_or(true),
                date,
                sessions,
                total_min,
                longest_min,
                longest_hm,
            }
        })
        .collect();
    days.sort_by(|a, b| a.date.cmp(&b.date));

    let total_sessions = days.iter().map(|d| d.sessions).sum();
    let total_min = days.iter().map(|d| d.total_min).sum();
    let longest_min = days.iter().map(|d| d.longest_min).max().unwrap_or(0);
    let best_day = days.iter().max_by_key(|d| d.total_min).cloned();

    Ok(FocusSummary {
        period_start: start_s,
        period_end: end_s,
        period_label: period_label_of(span, start),
        days,
        total_sessions,
        total_min,
        longest_min,
        best_day,
    })
}

/// 命令入口：周/月/年跨度聚合（快照模式同 get_bill，锁外查询）。
pub fn period_summary(hol: &HolidayCache, span: Span, offset: i64) -> Result<FocusSummary, String> {
    let today = Local::now().date_naive();
    let (start, end) = period_bounds(span, offset.max(0), today);
    db::with_db(|conn| summary_assemble(start, end, span, hol, conn))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;
    const THRESHOLD: i64 = 25;

    fn input(active: bool, work: Option<&str>, now_ms: i64) -> TickInput {
        TickInput {
            now_ms,
            active,
            work_app: work.map(String::from),
        }
    }

    #[test]
    fn qualifies_after_threshold_and_closes() {
        let mut m: Option<RunState> = None;
        // 25 分钟连续工作：起段 1 拍 + 延长 25 拍（每拍 1 分钟）
        assert!(tick_run(&mut m, input(true, Some("代码"), 0), THRESHOLD).is_none());
        for i in 1..=25 {
            assert!(tick_run(&mut m, input(true, Some("代码"), i * MIN), THRESHOLD).is_none());
        }
        // 断段：时长按最后确认活跃（25 分）计，断段拍本身不延长
        let s = tick_run(&mut m, input(false, None, 26 * MIN), THRESHOLD).unwrap();
        assert_eq!(s.minutes, 25);
        assert_eq!(s.top_app, "代码");
        assert!(m.is_none());
    }

    #[test]
    fn below_threshold_is_discarded() {
        let mut m: Option<RunState> = None;
        tick_run(&mut m, input(true, Some("代码"), 0), THRESHOLD);
        for i in 1..20 {
            tick_run(&mut m, input(true, Some("代码"), i * MIN), THRESHOLD);
        }
        // 仅 20 分钟就断段 → 无落库记录
        assert!(tick_run(&mut m, input(false, None, 20 * MIN), THRESHOLD).is_none());
        assert!(m.is_none());
    }

    #[test]
    fn slack_foreground_breaks_session() {
        let mut m: Option<RunState> = None;
        tick_run(&mut m, input(true, Some("代码"), 0), THRESHOLD);
        for i in 1..=30 {
            tick_run(&mut m, input(true, Some("代码"), i * MIN), THRESHOLD);
        }
        // 切到非工作类前台（work_app=None 即非工作）→ 立即结算
        let s = tick_run(&mut m, input(true, None, 31 * MIN), THRESHOLD).unwrap();
        assert_eq!(s.minutes, 30);
    }

    #[test]
    fn idle_gap_breaks_and_honest_minutes() {
        let mut m: Option<RunState> = None;
        tick_run(&mut m, input(true, Some("代码"), 0), THRESHOLD);
        for i in 1..=30 {
            tick_run(&mut m, input(true, Some("代码"), i * MIN), THRESHOLD);
        }
        // 有输入但前台非工作（开会）→ 断段；时长按最后确认活跃（30 分）计，
        // 中间 10 分钟空窗不计入——「诚实口径」
        let s = tick_run(&mut m, input(false, Some("微信"), 40 * MIN), THRESHOLD).unwrap();
        assert_eq!(s.minutes, 30);
        // 空窗之后重新起段：新段独立计时
        assert!(tick_run(&mut m, input(true, Some("代码"), 45 * MIN), THRESHOLD).is_none());
        assert!(m.is_some());
    }

    #[test]
    fn top_app_is_majority_app() {
        let mut m: Option<RunState> = None;
        tick_run(&mut m, input(true, Some("浏览器"), 0), THRESHOLD);
        for i in 1..=10 {
            tick_run(&mut m, input(true, Some("浏览器"), i * MIN), THRESHOLD);
        }
        for i in 11..=30 {
            tick_run(&mut m, input(true, Some("终端"), i * MIN), THRESHOLD);
        }
        let s = tick_run(&mut m, input(false, None, 31 * MIN), THRESHOLD).unwrap();
        assert_eq!(s.top_app, "终端");
    }

    #[test]
    fn threshold_boundary_inclusive() {
        // 24 分钟 < 25 → 丢弃
        let mut m: Option<RunState> = None;
        tick_run(&mut m, input(true, Some("代码"), 0), THRESHOLD);
        for i in 1..=24 {
            tick_run(&mut m, input(true, Some("代码"), i * MIN), THRESHOLD);
        }
        assert!(tick_run(&mut m, input(false, None, 24 * MIN), THRESHOLD).is_none());
        // 恰好 25 分钟整 → 结算（边界含）
        tick_run(&mut m, input(true, Some("代码"), 30 * MIN), THRESHOLD);
        for i in 31..=55 {
            tick_run(&mut m, input(true, Some("代码"), i * MIN), THRESHOLD);
        }
        let s = tick_run(&mut m, input(false, None, 56 * MIN), THRESHOLD).unwrap();
        assert_eq!(s.minutes, 25);
    }

    #[test]
    fn summary_groups_and_picks_best_day() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::CREATE_FOCUS_SESSIONS)
            .unwrap();
        for (d, minutes) in [("2026-09-07", 50), ("2026-09-07", 30), ("2026-09-08", 95)] {
            conn.execute(
                "INSERT INTO focus_sessions (date, start_hm, end_hm, minutes, top_app) \
                 VALUES (?1, '09:00', '10:00', ?2, '代码')",
                rusqlite::params![d, minutes],
            )
            .unwrap();
        }
        let hol = HolidayCache {
            year: 2026,
            ..Default::default()
        };
        let s = summary_assemble(
            NaiveDate::from_ymd_opt(2026, 9, 7).unwrap(),
            NaiveDate::from_ymd_opt(2026, 9, 13).unwrap(),
            Span::Week,
            &hol,
            &conn,
        )
        .unwrap();
        assert_eq!(s.period_label, "第 37 周");
        assert_eq!(s.days.len(), 2);
        assert_eq!(s.total_sessions, 3);
        assert_eq!(s.total_min, 50 + 30 + 95);
        assert_eq!(s.longest_min, 95);
        let d7 = &s.days[0];
        assert_eq!(d7.sessions, 2);
        assert_eq!(d7.total_min, 80);
        assert_eq!(d7.longest_min, 50);
        assert!(d7.longest_hm.as_deref() == Some("09:00–10:00"));
        assert_eq!(s.best_day.as_ref().unwrap().date, "2026-09-08");
    }
}
