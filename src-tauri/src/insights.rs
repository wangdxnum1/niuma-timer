//! 数据洞察聚合：时段热力图（7×24 键鼠/前台/音频）、多周趋势（复用周账单口径）、
//! 身体账单（act_hourly 周聚合）。口径延续 weekbill：
//! events = moves+left+dbl+right+wheel+mid+xbtn+keys；clicks = left−dbl+right+mid+xbtn。
//! 设计见 docs/plans/2026-09-18-v1.2.0-insights-design.md。

use std::collections::{BTreeMap, HashMap};

use chrono::{Datelike, Duration, Local, NaiveDate};
use rusqlite::Connection;
use serde::Serialize;

use crate::app_usage;
use crate::config::Config;
use crate::db;
use crate::holiday::HolidayCache;
use crate::weekbill::{is_workday_of, period_bill, period_bounds, Span};

const WEEKDAYS_CN: [&str; 7] = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];

fn weekday_cn(d: NaiveDate) -> &'static str {
    WEEKDAYS_CN[d.weekday().num_days_from_monday() as usize]
}

/// 键鼠总事件口径（与周账单 act_events 一致），SQL 里逐列 SUM 拼不出就传值算
const EVENTS_EXPR: &str =
    "SUM(moves)+SUM(`left`)+SUM(dbl)+SUM(`right`)+SUM(wheel)+SUM(mid)+SUM(xbtn)+SUM(keys)";

// ---------- 时段热力图 ----------

/// 单个小时代格（只含有效值，前端 sqrt 归一画 4 级色阶）
#[derive(Debug, Serialize)]
pub struct HourCell {
    pub date: String,
    /// 跨度坍缩后 date 为空串，行标签改用本字段
    pub weekday: String,
    pub hour: i64,
    /// 键鼠总事件（口径同周账单 act_events）
    pub events: i64,
    /// 该小时前台应用秒（app_usage_hourly 求和；监控关闭为 0）
    pub front_secs: i64,
    /// 该小时音频秒（audio_usage_hourly 求和；监控关闭为 0）
    pub audio_secs: i64,
}

#[derive(Debug, Serialize)]
pub struct HourHeatmap {
    pub week_start: String,
    pub week_end: String,
    /// 只含有数据的小时格，按 (date, hour) 有序
    pub cells: Vec<HourCell>,
    /// 本周最拼时段：跨天按小时聚合 events 最大者；无数据 None
    pub peak_hour: Option<i64>,
}

pub fn hour_heatmap_assemble(
    ws: NaiveDate,
    we: NaiveDate,
    conn: &Connection,
) -> rusqlite::Result<HourHeatmap> {
    let (a, b) = (ws.to_string(), we.to_string());
    let mut map: BTreeMap<(String, i64), HourCell> = BTreeMap::new();

    let mut st = conn.prepare(&format!(
        "SELECT date, hour, {EVENTS_EXPR} FROM act_hourly \
         WHERE date BETWEEN ?1 AND ?2 GROUP BY date, hour"
    ))?;
    let rows = st
        .query_map(rusqlite::params![a, b], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (date, hour, events) in rows {
        let weekday = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
            .map(weekday_cn)
            .unwrap_or("")
            .to_string();
        map.insert(
            (date.clone(), hour),
            HourCell {
                date,
                weekday,
                hour,
                events,
                front_secs: 0,
                audio_secs: 0,
            },
        );
    }

    // 前台 / 音频秒合并进同一批格（两张表同构：date+hour+app/seconds）
    for (sql, key) in [
        (
            "SELECT date, hour, SUM(seconds) FROM app_usage_hourly \
             WHERE date BETWEEN ?1 AND ?2 GROUP BY date, hour",
            0u8,
        ),
        (
            "SELECT date, hour, SUM(seconds) FROM audio_usage_hourly \
             WHERE date BETWEEN ?1 AND ?2 GROUP BY date, hour",
            1u8,
        ),
    ] {
        let mut st = conn.prepare(sql)?;
        let rows = st
            .query_map(rusqlite::params![a, b], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (date, hour, secs) in rows {
            if let Some(cell) = map.get_mut(&(date, hour)) {
                if key == 0 {
                    cell.front_secs = secs;
                } else {
                    cell.audio_secs = secs;
                }
            }
        }
    }

    // 最拼时段：跨天按小时聚合
    let mut by_hour: BTreeMap<i64, i64> = BTreeMap::new();
    for cell in map.values() {
        *by_hour.entry(cell.hour).or_insert(0) += cell.events;
    }
    let peak_hour = by_hour
        .into_iter()
        .filter(|(_, ev)| *ev > 0)
        .max_by_key(|(_, ev)| *ev)
        .map(|(h, _)| h);

    Ok(HourHeatmap {
        week_start: a,
        week_end: b,
        cells: map.into_values().collect(),
        peak_hour,
    })
}

pub fn hour_heatmap(span: Span, offset: i64) -> Result<HourHeatmap, String> {
    let today = Local::now().date_naive();
    let (ws, we) = period_bounds(span, offset, today);
    let hm = db::with_db(|conn| hour_heatmap_assemble(ws, we, conn))?;
    // 月/年视图不再有逐日格：坍缩成「星期 × 小时」矩阵，前端渲染路径不变
    Ok(match span {
        Span::Week => hm,
        _ => collapse_to_weekly_cells(hm),
    })
}

/// 月/年跨度 → 7×24 坍缩：把逐日格按 (星期几, 小时) 聚合。date 置空串，peak_hour 按聚合后 events 重算。
fn collapse_to_weekly_cells(hm: HourHeatmap) -> HourHeatmap {
    let mut agg: BTreeMap<(usize, i64), (i64, i64, i64)> = BTreeMap::new();
    for c in &hm.cells {
        let wd = WEEKDAYS_CN
            .iter()
            .position(|w| *w == c.weekday)
            .unwrap_or(0);
        let e = agg.entry((wd, c.hour)).or_insert((0, 0, 0));
        e.0 += c.events;
        e.1 += c.front_secs;
        e.2 += c.audio_secs;
    }
    let cells: Vec<HourCell> = agg
        .into_iter()
        .map(|((wd, hour), (events, front_secs, audio_secs))| HourCell {
            date: String::new(),
            weekday: WEEKDAYS_CN[wd].to_string(),
            hour,
            events,
            front_secs,
            audio_secs,
        })
        .collect();
    let mut by_hour: BTreeMap<i64, i64> = BTreeMap::new();
    for c in &cells {
        *by_hour.entry(c.hour).or_insert(0) += c.events;
    }
    let peak_hour = by_hour
        .into_iter()
        .filter(|(_, ev)| *ev > 0)
        .max_by_key(|(_, ev)| *ev)
        .map(|(h, _)| h);
    HourHeatmap {
        week_start: hm.week_start,
        week_end: hm.week_end,
        cells,
        peak_hour,
    }
}

// ---------- 多周趋势 ----------

#[derive(Debug, Serialize)]
pub struct TrendPoint {
    /// "第 37 周" / "2026 年 9 月" / "2026 年"
    pub label: String,
    pub period_start: String,
    /// 供前端 paintBillNav 与悬停区间直接使用
    pub period_end: String,
    /// 周期总入账（复用周期账单口径，与账单页数字严格一致）
    pub income: f64,
    /// 周期摸鱼率 = Σslack_seconds ÷ front_seconds；front=0 → None（前端断线不画）
    pub slack_rate: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct WeekTrend {
    /// 窗口最早一周的周一
    pub week_start: String,
    /// 旧→新 8 个点
    pub points: Vec<TrendPoint>,
}

/// 多周期趋势：以 offset 对应周期为最新点的最近 8 个周期，逐期复用 weekbill::period_bill。
/// 不再开 with_db：period_bill 自带连接管理，锁纪律由其保证。
pub fn period_trend(
    cfg: &Config,
    cur_hol: &HolidayCache,
    span: Span,
    offset: i64,
) -> Result<WeekTrend, String> {
    let off = offset.max(0);
    let today = Local::now().date_naive();
    let (earliest, _) = period_bounds(span, off + 7, today);
    let mut points = Vec::new();
    for i in 0..8 {
        let pb = period_bill(cfg, cur_hol, span, off + i)?;
        points.push(TrendPoint {
            label: pb.period_label,
            period_start: pb.period_start,
            period_end: pb.period_end,
            income: pb.total_income,
            slack_rate: slack_rate_of(
                pb.front_seconds,
                pb.buckets.iter().map(|b| b.slack_seconds).sum(),
            ),
        });
    }
    points.reverse();
    Ok(WeekTrend {
        week_start: earliest.to_string(),
        points,
    })
}

/// 周摸鱼率：front=0 → None（前端断线不画 0），否则 slack ÷ front
fn slack_rate_of(front: i64, slack: i64) -> Option<f64> {
    if front > 0 {
        Some(slack as f64 / front as f64)
    } else {
        None
    }
}

// ---------- 身体账单 ----------

#[derive(Debug, Serialize, PartialEq)]
pub struct BodyDay {
    pub date: String,
    pub weekday: String,
    pub events: i64,
}

#[derive(Debug, Serialize)]
pub struct BodyBill {
    pub week_start: String,
    pub week_end: String,
    /// 有活动的真实天数；年视图按月分桶后仍用于日均。
    pub record_days: usize,
    /// 点击 = left − dbl + right + mid + xbtn（双击折算 1 次，同周账单口径）
    pub clicks: i64,
    pub keys: i64,
    pub moves: i64,
    /// 鼠标滑动像素，前端按 96dpi 换算米
    pub pixels: i64,
    pub wheel_ticks: i64,
    /// 恒 7 天（无记录天 events=0）
    pub days: Vec<BodyDay>,
    /// 最累日 = events 最大且 > 0
    pub busiest: Option<BodyDay>,
}

pub fn body_bill_assemble(
    ws: NaiveDate,
    we: NaiveDate,
    conn: &Connection,
) -> rusqlite::Result<BodyBill> {
    let (a, b) = (ws.to_string(), we.to_string());

    // 按天事件数
    let mut by_day: BTreeMap<String, i64> = BTreeMap::new();
    let mut st = conn.prepare(&format!(
        "SELECT date, {EVENTS_EXPR} FROM act_hourly \
         WHERE date BETWEEN ?1 AND ?2 GROUP BY date"
    ))?;
    let rows = st
        .query_map(rusqlite::params![a, b], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (date, events) in rows {
        by_day.insert(date, events);
    }

    // 全区间五指标（COALESCE：空周全 0 而不是 NULL）
    let (clicks, keys, moves, pixels, wheel_ticks) = conn.query_row(
        "SELECT COALESCE(SUM(`left`)-SUM(dbl)+SUM(`right`)+SUM(mid)+SUM(xbtn),0), \
                COALESCE(SUM(keys),0), COALESCE(SUM(moves),0), \
                COALESCE(SUM(pixels),0), COALESCE(SUM(wheel_ticks),0) \
         FROM act_hourly WHERE date BETWEEN ?1 AND ?2",
        rusqlite::params![a, b],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )?;

    let mut days = Vec::new();
    let mut d = ws;
    while d <= we {
        let ds = d.to_string();
        days.push(BodyDay {
            events: *by_day.get(&ds).unwrap_or(&0),
            date: ds,
            weekday: weekday_cn(d).to_string(),
        });
        d += Duration::days(1);
    }
    let busiest = days
        .iter()
        .filter(|d| d.events > 0)
        .max_by_key(|d| d.events)
        .map(|d| BodyDay {
            date: d.date.clone(),
            weekday: d.weekday.clone(),
            events: d.events,
        });

    Ok(BodyBill {
        week_start: a,
        week_end: b,
        record_days: days.iter().filter(|d| d.events > 0).count(),
        clicks,
        keys,
        moves,
        pixels,
        wheel_ticks,
        days,
        busiest,
    })
}

pub fn body_bill(span: Span, offset: i64) -> Result<BodyBill, String> {
    let today = Local::now().date_naive();
    let (ws, we) = period_bounds(span, offset, today);
    let bb = db::with_db(|conn| body_bill_assemble(ws, we, conn))?;
    Ok(match span {
        Span::Year => collapse_days_to_months(bb),
        _ => bb,
    })
}

/// 年视图坍缩：逐日 BodyDay → 12 个月桶（date = "2026-09"，weekday = "9 月"），busiest 重算
fn collapse_days_to_months(bb: BodyBill) -> BodyBill {
    let mut by_month: BTreeMap<String, i64> = BTreeMap::new();
    for d in &bb.days {
        if let Some(ym) = d.date.get(..7) {
            *by_month.entry(ym.to_string()).or_insert(0) += d.events;
        }
    }
    let year = bb.week_start.get(..4).unwrap_or("").to_string();
    let days: Vec<BodyDay> = (1u32..=12)
        .map(|m| {
            let ym = format!("{year}-{m:02}");
            BodyDay {
                events: *by_month.get(&ym).unwrap_or(&0),
                date: ym,
                weekday: format!("{m} 月"),
            }
        })
        .collect();
    let busiest = days
        .iter()
        .filter(|d| d.events > 0)
        .max_by_key(|d| d.events)
        .map(|d| BodyDay {
            date: d.date.clone(),
            weekday: d.weekday.clone(),
            events: d.events,
        });
    BodyBill {
        days,
        busiest,
        ..bb
    }
}

// ---------- 时间线回顾（v1.6.0） ----------

/// 单小时时间线行。无前台记录的小时各秒为 0——监控关闭 ≠ 离开，
/// 不臆造「离开」状态（延续「断线不画 0」的诚实数据原则）。
#[derive(Debug, Serialize)]
pub struct DayTimelineHour {
    pub hour: i64,
    /// 前台秒按四分类拆分（查询时归类，改分类即重算历史）
    pub work_secs: i64,
    pub slack_secs: i64,
    pub comm_secs: i64,
    pub other_secs: i64,
    /// 四类之和
    pub front_secs: i64,
    /// 该小时媒体播放秒
    pub audio_secs: i64,
    /// 键鼠事件（EVENTS_EXPR 口径）
    pub events: i64,
    /// 该小时前台秒最大的应用（仅展示，不出本机）
    pub top_app: Option<String>,
}

/// 一天的时间线回顾（24 行固定，渲染端无需补行）
#[derive(Debug, Serialize)]
pub struct DayTimeline {
    pub date: String,
    pub weekday: String,
    pub is_today: bool,
    pub is_workday: bool,
    pub total_front: i64,
    pub total_events: i64,
    pub hours: Vec<DayTimelineHour>,
}

/// 单日 24 小时聚合：app_usage_hourly 按 (hour, app) 归四类、act_hourly 按小时、
/// audio_usage_hourly 按小时。三表单日各一条 SQL，内存合并进 24 行。
pub fn day_timeline_assemble(
    date: NaiveDate,
    cfg: &Config,
    hol: &HolidayCache,
    conn: &Connection,
) -> rusqlite::Result<DayTimeline> {
    let date_s = date.format("%Y-%m-%d").to_string();
    let mut hours: Vec<DayTimelineHour> = (0..24)
        .map(|h| DayTimelineHour {
            hour: h,
            work_secs: 0,
            slack_secs: 0,
            comm_secs: 0,
            other_secs: 0,
            front_secs: 0,
            audio_secs: 0,
            events: 0,
            top_app: None,
        })
        .collect();

    // 前台秒按 (hour, app) 聚合后归类；同时记每小时前台秒最大的应用
    let mut top: HashMap<i64, (i64, String)> = HashMap::new();
    {
        let mut st = conn.prepare(
            "SELECT hour, app, SUM(seconds) FROM app_usage_hourly \
             WHERE date = ?1 GROUP BY hour, app",
        )?;
        let rows = st
            .query_map(rusqlite::params![date_s], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (hour, app, secs) in rows {
            if !(0..24).contains(&hour) {
                continue;
            }
            let h = &mut hours[hour as usize];
            let cat = app_usage::category_of(&app, cfg);
            if cat == app_usage::CAT_WORK {
                h.work_secs += secs;
            } else if cat == app_usage::CAT_SLACK {
                h.slack_secs += secs;
            } else if cat == app_usage::CAT_COMM {
                h.comm_secs += secs;
            } else {
                h.other_secs += secs;
            }
            h.front_secs += secs;
            let e = top.entry(hour).or_insert((0, String::new()));
            if secs > e.0 {
                *e = (secs, app);
            }
        }
    }
    for (hour, (_, app)) in top {
        hours[hour as usize].top_app = Some(app);
    }

    // 键鼠事件（口径同周账单 act_events）
    {
        let mut st = conn.prepare(&format!(
            "SELECT hour, {EVENTS_EXPR} FROM act_hourly WHERE date = ?1 GROUP BY hour"
        ))?;
        let rows = st
            .query_map(rusqlite::params![date_s], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (hour, events) in rows {
            if (0..24).contains(&hour) {
                hours[hour as usize].events = events;
            }
        }
    }

    // 媒体播放秒
    {
        let mut st = conn.prepare(
            "SELECT hour, SUM(seconds) FROM audio_usage_hourly WHERE date = ?1 GROUP BY hour",
        )?;
        let rows = st
            .query_map(rusqlite::params![date_s], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (hour, secs) in rows {
            if (0..24).contains(&hour) {
                hours[hour as usize].audio_secs = secs;
            }
        }
    }

    let total_front = hours.iter().map(|h| h.front_secs).sum();
    let total_events = hours.iter().map(|h| h.events).sum();
    Ok(DayTimeline {
        date: date_s,
        weekday: weekday_cn(date).to_string(),
        is_today: date == Local::now().date_naive(),
        is_workday: is_workday_of(date, hol),
        total_front,
        total_events,
        hours,
    })
}

// ---------- 单测（in-memory，仿 weekbill 基建） ----------

#[cfg(test)]
mod tests {
    use super::*;

    /// 时间线聚合：四类归口、top_app、键鼠/媒体并入、空小时全 0
    #[test]
    fn day_timeline_aggregates_categories() {
        let conn = mem_conn();
        let cfg = Config {
            app_categories: HashMap::from([
                ("代码".to_string(), "工作".to_string()),
                ("摸鱼网".to_string(), "摸鱼".to_string()),
                ("微信".to_string(), "沟通".to_string()),
            ]),
            ..Default::default()
        };
        let date = NaiveDate::from_ymd_opt(2026, 9, 7).unwrap(); // 周一
        conn.execute(
            "INSERT INTO app_usage_hourly (date, hour, app, seconds) VALUES ('2026-09-07', 9, '代码', 1800)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO app_usage_hourly (date, hour, app, seconds) VALUES ('2026-09-07', 9, '摸鱼网', 600)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO app_usage_hourly (date, hour, app, seconds) VALUES ('2026-09-07', 10, '微信', 900)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO app_usage_hourly (date, hour, app, seconds) VALUES ('2026-09-07', 15, '神秘软件', 300)",
            [],
        )
        .unwrap();
        insert_act(&conn, "2026-09-07", 9, [1, 0, 10, 0, 0, 0, 0, 0, 0]);
        conn.execute(
            "INSERT INTO audio_usage_hourly (date, hour, app, seconds) VALUES ('2026-09-07', 9, '网易云', 120)",
            [],
        )
        .unwrap();

        let empty_hol = HolidayCache {
            year: 2026,
            ..Default::default()
        };
        let tl = day_timeline_assemble(date, &cfg, &empty_hol, &conn).unwrap();
        assert_eq!(tl.hours.len(), 24, "固定 24 行");
        assert_eq!(tl.weekday, "周一");
        assert!(tl.is_workday);
        assert!(!tl.is_today, "过去日期不标今天");

        let h9 = &tl.hours[9];
        assert_eq!(h9.work_secs, 1800);
        assert_eq!(h9.slack_secs, 600);
        assert_eq!(h9.front_secs, 2400);
        assert_eq!(h9.top_app.as_deref(), Some("代码"), "前台秒最大者");
        assert!(h9.events > 0);
        assert_eq!(h9.audio_secs, 120);
        let h10 = &tl.hours[10];
        assert_eq!(h10.comm_secs, 900);
        let h15 = &tl.hours[15];
        assert_eq!(h15.other_secs, 300, "未分类归其他");
        assert_eq!(h15.top_app.as_deref(), Some("神秘软件"));
        let h23 = &tl.hours[23];
        assert_eq!(h23.front_secs, 0, "无记录小时全 0");
        assert_eq!(h23.top_app, None);
        assert_eq!(tl.total_front, 1800 + 600 + 900 + 300);
    }

    #[test]
    fn annual_body_preserves_recorded_day_count() {
        let conn = mem_conn();
        for day in 1..=20 {
            insert_act(
                &conn,
                &format!("2026-01-{day:02}"),
                10,
                [1, 0, 10, 0, 0, 0, 0, 0, 0],
            );
        }
        let bb = body_bill_assemble(
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
            &conn,
        )
        .unwrap();
        let year = collapse_days_to_months(bb);
        assert_eq!(year.record_days, 20);
        assert_eq!(year.days[0].weekday, "1 月");
    }
    use rusqlite::Connection;

    use crate::db::{CREATE_ACT_HOURLY, CREATE_APP_USAGE_HOURLY, CREATE_AUDIO_USAGE_HOURLY};

    /// 2026-09-07 是周一，取 09-07..09-13 一整周
    fn week() -> (NaiveDate, NaiveDate) {
        (
            NaiveDate::from_ymd_opt(2026, 9, 7).unwrap(),
            NaiveDate::from_ymd_opt(2026, 9, 13).unwrap(),
        )
    }

    fn mem_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(CREATE_ACT_HOURLY).unwrap();
        conn.execute_batch(CREATE_APP_USAGE_HOURLY).unwrap();
        conn.execute_batch(CREATE_AUDIO_USAGE_HOURLY).unwrap();
        conn
    }

    fn insert_act(conn: &Connection, date: &str, hour: i64, ev: [i64; 9]) {
        // [moves, pixels, left, dbl, right, wheel, wheel_ticks, mid, xbtn] + keys 单独
        conn.execute(
            "INSERT INTO act_hourly (date, hour, moves, pixels, `left`, dbl, `right`, wheel, wheel_ticks, mid, xbtn, keys) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![date, hour, ev[0], ev[1], ev[2], ev[3], ev[4], ev[5], ev[6], ev[7], ev[8], 200],
        )
        .unwrap();
    }

    #[test]
    fn heatmap_aggregates_three_sources() {
        let conn = mem_conn();
        let (ws, we) = week();
        // events = 100+10+2+3+50+1+0+200 = 366（keys 固定插 200；wheel_ticks 不计入口径）
        insert_act(&conn, "2026-09-07", 9, [100, 500, 10, 2, 3, 50, 5, 1, 0]);
        conn.execute(
            "INSERT INTO app_usage_hourly (date, hour, app, seconds) VALUES ('2026-09-07', 9, 'idea', 1800)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO audio_usage_hourly (date, hour, app, seconds) VALUES ('2026-09-07', 9, 'spotify', 600)",
            [],
        )
        .unwrap();

        let hm = hour_heatmap_assemble(ws, we, &conn).unwrap();
        assert_eq!(hm.cells.len(), 1);
        let c = &hm.cells[0];
        assert_eq!((c.date.as_str(), c.hour), ("2026-09-07", 9));
        assert_eq!(c.events, 366);
        assert_eq!(c.front_secs, 1800);
        assert_eq!(c.audio_secs, 600);
        assert_eq!(hm.peak_hour, Some(9));
        assert_eq!(hm.week_start, "2026-09-07");
    }

    #[test]
    fn heatmap_peak_across_days() {
        let conn = mem_conn();
        let (ws, we) = week();
        // 9 点两天共 250+250=500，10 点一天 600 → peak = 10
        insert_act(&conn, "2026-09-07", 9, [50, 0, 0, 0, 0, 0, 0, 0, 0]);
        insert_act(&conn, "2026-09-08", 9, [50, 0, 0, 0, 0, 0, 0, 0, 0]);
        insert_act(&conn, "2026-09-08", 10, [400, 0, 0, 0, 0, 0, 0, 0, 0]);
        let hm = hour_heatmap_assemble(ws, we, &conn).unwrap();
        assert_eq!(hm.cells.len(), 3);
        assert_eq!(hm.peak_hour, Some(10));
    }

    #[test]
    fn heatmap_empty_week() {
        let conn = mem_conn();
        let (ws, we) = week();
        let hm = hour_heatmap_assemble(ws, we, &conn).unwrap();
        assert!(hm.cells.is_empty());
        assert_eq!(hm.peak_hour, None);
    }

    #[test]
    fn body_clicks_and_totals() {
        let conn = mem_conn();
        let (ws, we) = week();
        // clicks = 10 − 2 + 3 + 1 + 0 = 12；keys=200、moves=100、pixels=500、wheel_ticks=5
        insert_act(&conn, "2026-09-07", 9, [100, 500, 10, 2, 3, 50, 5, 1, 0]);
        let bb = body_bill_assemble(ws, we, &conn).unwrap();
        assert_eq!(bb.clicks, 12);
        assert_eq!(bb.keys, 200);
        assert_eq!(bb.moves, 100);
        assert_eq!(bb.pixels, 500);
        assert_eq!(bb.wheel_ticks, 5);
        // 恒 7 天且首个是周一
        assert_eq!(bb.days.len(), 7);
        assert_eq!(bb.days[0].date, "2026-09-07");
        assert_eq!(bb.days[0].weekday, "周一");
        assert_eq!(bb.days[6].date, "2026-09-13");
        assert_eq!(bb.busiest.as_ref().unwrap().weekday, "周一");
    }

    #[test]
    fn body_empty_week() {
        let conn = mem_conn();
        let (ws, we) = week();
        let bb = body_bill_assemble(ws, we, &conn).unwrap();
        assert_eq!(bb.clicks, 0);
        assert_eq!(bb.days.len(), 7);
        assert!(bb.days.iter().all(|d| d.events == 0));
        assert_eq!(bb.busiest, None);
    }

    #[test]
    fn slack_rate_rules() {
        assert_eq!(slack_rate_of(0, 1200), None); // 无前台秒不评判
        let r = slack_rate_of(4800, 1200).unwrap();
        assert!((r - 0.25).abs() < 1e-9);
    }

    #[test]
    fn trend_window_is_eight_periods() {
        // 8 个周点 = 最新周起点往前 7 周：跨度 49 天
        let today = NaiveDate::from_ymd_opt(2026, 9, 23).unwrap();
        let (a, _) = period_bounds(Span::Week, 0, today);
        let (b, _) = period_bounds(Span::Week, 7, today);
        assert_eq!((a - b).num_days(), 49);
    }

    #[test]
    fn future_week_bounds_are_capped() {
        // 名字保留：验证周负偏移封顶
        let today = NaiveDate::from_ymd_opt(2026, 9, 23).unwrap();
        assert_eq!(
            period_bounds(Span::Week, -5, today),
            period_bounds(Span::Week, 0, today)
        );
        let (s, e) = period_bounds(Span::Week, 0, today);
        assert_eq!((e - s).num_days(), 6);
    }
}
