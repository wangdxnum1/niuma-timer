//! 数据洞察聚合：时段热力图（7×24 键鼠/前台/音频）、多周趋势（复用周账单口径）、
//! 身体账单（act_hourly 周聚合）。口径延续 weekbill：
//! events = moves+left+dbl+right+wheel+mid+xbtn+keys；clicks = left−dbl+right+mid+xbtn。
//! 设计见 docs/plans/2026-09-18-v1.2.0-insights-design.md。

use std::collections::BTreeMap;

use chrono::{Datelike, Duration, Local, NaiveDate};
use rusqlite::Connection;
use serde::Serialize;

use crate::config::Config;
use crate::db;
use crate::holiday::HolidayCache;
use crate::weekbill::{week_bill, week_start_of};

const WEEKDAYS_CN: [&str; 7] = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];

fn weekday_cn(d: NaiveDate) -> &'static str {
    WEEKDAYS_CN[d.weekday().num_days_from_monday() as usize]
}

/// 键鼠总事件口径（与周账单 act_events 一致），SQL 里逐列 SUM 拼不出就传值算
const EVENTS_EXPR: &str = "SUM(moves)+SUM(`left`)+SUM(dbl)+SUM(`right`)+SUM(wheel)+SUM(mid)+SUM(xbtn)+SUM(keys)";

// ---------- 时段热力图 ----------

/// 单个小时代格（只含有效值，前端 sqrt 归一画 4 级色阶）
#[derive(Debug, Serialize)]
pub struct HourCell {
    pub date: String,
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
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (date, hour, events) in rows {
        map.insert(
            (date.clone(), hour),
            HourCell { date, hour, events, front_secs: 0, audio_secs: 0 },
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
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
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

pub fn hour_heatmap(week_offset: i64) -> Result<HourHeatmap, String> {
    let (ws, we) = week_bounds(week_offset);
    db::with_db(|conn| hour_heatmap_assemble(ws, we, conn))
}

// ---------- 多周趋势 ----------

#[derive(Debug, Serialize)]
pub struct TrendPoint {
    pub week_start: String,
    /// 周总入账（复用周账单口径，与账单页数字严格一致）
    pub income: f64,
    /// 周摸鱼率 = Σslack_seconds ÷ front_seconds；front=0 → None（前端断线不画）
    pub slack_rate: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct WeekTrend {
    /// 窗口最早一周的周一
    pub week_start: String,
    /// 旧→新 8 个点
    pub points: Vec<TrendPoint>,
}

/// 以 week_offset 对应周为最新点的最近 8 周，逐周复用 weekbill::week_bill。
/// 不再开 with_db：week_bill 自带连接管理，锁纪律由其保证。
pub fn week_trend(
    cfg: &Config,
    cur_hol: &HolidayCache,
    week_offset: i64,
) -> Result<WeekTrend, String> {
    let off = week_offset.max(0);
    let (earliest, _) = week_bounds(off + 7);
    let mut points = Vec::with_capacity(8);
    for i in 0..8 {
        let wb = week_bill(cfg, cur_hol, off + i)?;
        points.push(TrendPoint {
            week_start: wb.week_start,
            income: wb.total_income,
            slack_rate: slack_rate_of(wb.front_seconds, wb.days.iter().map(|d| d.slack_seconds).sum()),
        });
    }
    points.reverse(); // 收集顺序是最新→最旧，翻转成旧→新
    Ok(WeekTrend { week_start: earliest.to_string(), points })
}

/// 周摸鱼率：front=0 → None（前端断线不画 0），否则 slack ÷ front
fn slack_rate_of(front: i64, slack: i64) -> Option<f64> {
    if front > 0 { Some(slack as f64 / front as f64) } else { None }
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

    let mut days = Vec::with_capacity(7);
    for i in 0..7 {
        let d = ws + Duration::days(i);
        let ds = d.to_string();
        days.push(BodyDay {
            events: *by_day.get(&ds).unwrap_or(&0),
            date: ds,
            weekday: weekday_cn(d).to_string(),
        });
    }
    let busiest = days
        .iter()
        .filter(|d| d.events > 0)
        .max_by_key(|d| d.events)
        .map(|d| BodyDay { date: d.date.clone(), weekday: d.weekday.clone(), events: d.events });

    Ok(BodyBill {
        week_start: a,
        week_end: b,
        clicks,
        keys,
        moves,
        pixels,
        wheel_ticks,
        days,
        busiest,
    })
}

pub fn body_bill(week_offset: i64) -> Result<BodyBill, String> {
    let (ws, we) = week_bounds(week_offset);
    db::with_db(|conn| body_bill_assemble(ws, we, conn))
}

// ---------- 共用 ----------

/// 周区间：offset 封顶 0（未来周回本周），ws 对齐周一，we = ws+6
fn week_bounds(week_offset: i64) -> (NaiveDate, NaiveDate) {
    let today = Local::now().date_naive();
    let ws = week_start_of(today - Duration::weeks(week_offset.max(0)));
    (ws, ws + Duration::days(6))
}

// ---------- 单测（in-memory，仿 weekbill 基建） ----------

#[cfg(test)]
mod tests {
    use super::*;
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
    fn trend_window_is_eight_weeks() {
        // 窗口结构：earliest(off+7) 与最新周相差 49 天；点序旧→新由 reverse 保证，
        // week_bill 循环本身碰真实库，数值行为交由 build 后冒烟覆盖（weekbill 同纪律）
        let (newest, _) = week_bounds(0);
        let (earliest, _) = week_bounds(7);
        assert_eq!((newest - earliest).num_days(), 49);
    }

    #[test]
    fn week_bounds_caps_future() {
        let (ws_neg, _) = week_bounds(-5);
        let (ws_zero, _) = week_bounds(0);
        assert_eq!(ws_neg, ws_zero); // 未来周封顶本周
        let we = week_bounds(0).1;
        assert_eq!((we - ws_zero).num_days(), 6);
    }
}
