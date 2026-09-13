//! 周账单聚合：把一周的加班流水、活动计数、应用分类秒折成一张 WeekBill。
//! 口径见 docs/plans/2026-09-13-week-bill-design.md（口径 1–9）：
//! 满勤工资按天取当月分母、摸鱼成本按天折算、total_income 不减摸鱼成本、
//! 环比同函数聚合上一周、跨年周按天的年份取内置节假日表且不触发网络。

use std::collections::HashMap;

use chrono::{Datelike, Duration, Local, NaiveDate, Weekday};
use rusqlite::Connection;
use serde::Serialize;

use crate::app_usage;
use crate::calc;
use crate::config::{self, Config};
use crate::db;
use crate::holiday::{self, HolidayCache};

/// 单日账单（前端渲染小票金条 / 双柱图的最小颗粒）
#[derive(Debug, Serialize)]
pub struct DayBill {
    /// "2026-09-07"
    pub date: String,
    /// "周一"
    pub weekday: String,
    /// 法定口径（节假日表 → 周几兜底）
    pub is_workday: bool,
    /// 应赚工资：今天 = 实时 earned、过去工作日 = 满勤、未来/休息日 = 0
    pub salary: f64,
    /// 当日加班费合计（ot_records.total，fee+meal 已落盘）
    pub ot_total: f64,
    /// 当日摸鱼秒（分类=摸鱼 的应用前台秒）
    pub slack_seconds: i64,
    /// 当日事件总数口径 = moves+left+dbl+right+wheel+mid+xbtn+keys
    pub act_events: i64,
    pub keys: i64,
    /// 点击口径 = (left−2×dbl)+dbl+right+mid+xbtn，滚轮不计
    pub clicks: i64,
    /// 当日摸鱼率 0–1（摸鱼秒 ÷ 四类前台总秒），无前台秒为 0
    pub slack_rate: f64,
    /// 有任意监控证据（act_events>0 或应用秒>0）
    pub has_record: bool,
}

/// 最累日（加班 valid_hours 最大）
#[derive(Debug, Serialize)]
pub struct HardestDay {
    pub date: String,
    pub weekday: String,
    pub ot_hours: f64,
}

/// 最摸日（slack_rate 最大且 > 0）
#[derive(Debug, Serialize)]
pub struct SlackiestDay {
    pub date: String,
    pub weekday: String,
    pub rate: f64,
}

/// 一周账单。字段名 snake_case 直达前端。
#[derive(Debug, Serialize)]
pub struct WeekBill {
    /// "2026-09-07"
    pub week_start: String,
    /// "2026-09-13"
    pub week_end: String,
    /// "第 37 周"
    pub week_no: String,
    pub is_current_week: bool,
    /// 总入账 = base_salary + ot_fee（摸鱼成本只展示、不做减法）
    pub total_income: f64,
    /// 应赚工资（含今天实时 earned）
    pub base_salary: f64,
    pub ot_fee: f64,
    /// 摸鱼成本 = Σ(当日摸鱼秒/3600 × 当日时薪)，按天折算
    pub slack_cost: f64,
    pub prev_total: f64,
    /// 百分点 ((cur−prev)/prev×100)；prev≤0 → None（前端不显示箭头）
    pub delta_pct: Option<f64>,
    /// 有活动或应用记录的工作日数；活动与应用监控都关 → null（前端显示 "—"）
    pub work_days: Option<i64>,
    /// 排班口径估算 = 出勤工作日 × daily_hours + 加班 valid_hours
    pub work_hours: f64,
    pub keys_total: i64,
    pub clicks_total: i64,
    /// 周摸鱼率（分子分母均只含工作日）
    pub slack_rate: f64,
    /// 工作日四类前台总秒；>0 才有金句资格（数据不足不评判）
    pub front_seconds: i64,
    pub hardest: Option<HardestDay>,
    pub slackiest: Option<SlackiestDay>,
    pub days: Vec<DayBill>,
}

/// 汇聚组装所需的全部锁外快照（config/holiday 由调用方取副本，避免锁内做 DB 查询）
pub struct WeekInput<'a> {
    pub cfg: &'a Config,
    pub cur_hol: &'a HolidayCache,
    pub week_start: NaiveDate,
    pub today: NaiveDate,
    /// calc::compute().earned，调用方算好传入
    pub today_earned: f64,
}

const WEEKDAYS_CN: [&str; 7] = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];

fn weekday_cn(d: NaiveDate) -> &'static str {
    WEEKDAYS_CN[d.weekday().num_days_from_monday() as usize]
}

/// 对齐到周一
pub fn week_start_of(d: NaiveDate) -> NaiveDate {
    d - Duration::days(d.weekday().num_days_from_monday() as i64)
}

/// 环比百分点；上周无基数（≤0）→ None，前端不显示箭头
pub fn delta_pct(cur: f64, prev: f64) -> Option<f64> {
    if prev <= 0.0 {
        None
    } else {
        Some((cur - prev) / prev * 100.0)
    }
}

/// 法定工作日口径：当天年份若非当前缓存年份，取内置表（owned，解决生命周期）；
/// 表缺失/为空退回周一至五；不触发网络刷新。
fn is_workday_of(date: NaiveDate, cur: &HolidayCache) -> bool {
    let local: Option<HolidayCache> = if cur.year == date.year() {
        None
    } else {
        holiday::builtin_cache(date.year())
    };
    let hol = local.as_ref().unwrap_or(cur);
    match hol.is_workday(date) {
        Some(v) => v,
        None => date.weekday() != Weekday::Sat && date.weekday() != Weekday::Sun,
    }
}

/// 当月工作日分母：override > 节假日表 > weekday_count（跨年同 is_workday_of 处理）
fn monthly_workdays_of(
    year: i32,
    month: u32,
    cfg: &Config,
    cur: &HolidayCache,
) -> u32 {
    if let Some(v) = config::effective_workdays_override(cfg, year, month) {
        return v;
    }
    let local: Option<HolidayCache> = if cur.year == year {
        None
    } else {
        holiday::builtin_cache(year)
    };
    let hol = local.as_ref().unwrap_or(cur);
    hol.month_workdays(year, month)
        .unwrap_or_else(|| holiday::weekday_count(year, month))
}

/// 一周聚合（conn 由调用方给：正式走 with_db，单测走 in-memory）
pub fn assemble(input: &WeekInput, conn: &Connection) -> rusqlite::Result<WeekBill> {
    let ws = input.week_start;
    let we = ws + Duration::days(6);
    let start_s = ws.format("%Y-%m-%d").to_string();
    let end_excl_s = (we + Duration::days(1)).format("%Y-%m-%d").to_string();
    let daily_h = calc::daily_hours(input.cfg);

    // ---- 加班流水：SUM(total) 与 SUM(valid_hours) 按天（不按费率重算，跨月天然正确） ----
    let mut ot_map: HashMap<String, (f64, f64)> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT date, SUM(total), SUM(valid_hours) \
             FROM ot_records WHERE date >= ?1 AND date < ?2 GROUP BY date",
        )?;
        let rows = stmt.query_map(rusqlite::params![start_s, end_excl_s], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, f64>(2)?,
            ))
        })?;
        for row in rows {
            let (d, total, vh) = row?;
            ot_map.insert(d, (total, vh));
        }
    }

    // ---- 活动计数：total_events 口径 + 键 + 点击（(left−2×dbl)+dbl+right+mid+xbtn） ----
    let mut act_map: HashMap<String, (i64, i64, i64)> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT date, \
                    SUM(moves)+SUM(`left`)+SUM(dbl)+SUM(`right`)+SUM(wheel)+SUM(mid)+SUM(xbtn)+SUM(keys), \
                    SUM(keys), \
                    SUM(`left`)-SUM(dbl)+SUM(`right`)+SUM(mid)+SUM(xbtn) \
             FROM act_hourly WHERE date >= ?1 AND date < ?2 GROUP BY date",
        )?;
        let rows = stmt.query_map(rusqlite::params![start_s, end_excl_s], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        for row in rows {
            let (d, events, keys, clicks) = row?;
            act_map.insert(d, (events, keys, clicks.max(0)));
        }
    }

    // ---- 应用前台秒：按天累计四类总秒与摸鱼秒（查询时归类，零迁移） ----
    let mut app_map: HashMap<String, (i64, i64)> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT date, app, SUM(seconds) \
             FROM app_usage WHERE date >= ?1 AND date < ?2 GROUP BY date, app",
        )?;
        let rows = stmt.query_map(rusqlite::params![start_s, end_excl_s], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        for row in rows {
            let (d, app, secs) = row?;
            let e = app_map.entry(d).or_insert((0, 0));
            e.0 += secs;
            if app_usage::category_of(&app, input.cfg) == app_usage::CAT_SLACK {
                e.1 += secs;
            }
        }
    }

    // ---- 逐日组装 ----
    let mut days: Vec<DayBill> = Vec::with_capacity(7);
    let mut base_salary = 0.0;
    let mut ot_fee = 0.0;
    let mut slack_cost = 0.0;
    let mut ot_hours_total = 0.0;
    let mut keys_total = 0i64;
    let mut clicks_total = 0i64;
    let mut slack_work = 0i64; // 周摸鱼率分子（仅工作日）
    let mut front_work = 0i64; // 周摸鱼率分母（仅工作日）
    let mut work_days = 0i64;
    let mut hardest: Option<HardestDay> = None;
    let mut slackiest: Option<SlackiestDay> = None;

    for i in 0..7 {
        let d = ws + Duration::days(i);
        let key = d.format("%Y-%m-%d").to_string();
        let is_wd = is_workday_of(d, input.cur_hol);
        let mw = monthly_workdays_of(d.year(), d.month(), input.cfg, input.cur_hol);
        let full_salary = if input.cfg.monthly_salary > 0.0 && mw > 0 {
            input.cfg.monthly_salary / mw as f64
        } else {
            0.0
        };

        // 工资：休息日 0；今天实时；过去工作日满勤；未来不预支
        let salary = if !is_wd {
            0.0
        } else if d == input.today {
            input.today_earned
        } else if d < input.today {
            full_salary
        } else {
            0.0
        };

        let (ot_total, ot_hours) = ot_map
            .get(&key)
            .map(|t| (t.0, t.1))
            .unwrap_or((0.0, 0.0));
        let (act_events, keys, clicks) = act_map.get(&key).copied().unwrap_or((0, 0, 0));
        let (front, slack) = app_map.get(&key).copied().unwrap_or((0, 0));
        let has_record = act_events > 0 || front > 0;

        // 当日时薪 = 满勤 ÷ 当日工时（固定时薪；今天实时 earned 只影响 salary 字段）
        let day_rate = if daily_h > 0.0 { full_salary / daily_h } else { 0.0 };
        // 摸鱼成本按天折算：休息日/未配月薪 → 0
        let day_slack_cost = if is_wd && day_rate > 0.0 {
            slack as f64 / 3600.0 * day_rate
        } else {
            0.0
        };

        let rate = if front > 0 {
            slack as f64 / front as f64
        } else {
            0.0
        };

        if is_wd && has_record {
            work_days += 1;
            slack_work += slack;
            front_work += front;
        }

        base_salary += salary;
        ot_fee += ot_total;
        ot_hours_total += ot_hours;
        slack_cost += day_slack_cost;
        keys_total += keys;
        clicks_total += clicks;

        if ot_hours > 0.0 && hardest.as_ref().map_or(true, |h| ot_hours > h.ot_hours) {
            hardest = Some(HardestDay {
                date: key.clone(),
                weekday: weekday_cn(d).into(),
                ot_hours,
            });
        }
        if has_record && rate > 0.0 && slackiest.as_ref().map_or(true, |s| rate > s.rate) {
            slackiest = Some(SlackiestDay {
                date: key.clone(),
                weekday: weekday_cn(d).into(),
                rate,
            });
        }

        days.push(DayBill {
            date: key,
            weekday: weekday_cn(d).into(),
            is_workday: is_wd,
            salary,
            ot_total,
            slack_seconds: slack,
            act_events,
            keys,
            clicks,
            slack_rate: rate,
            has_record,
        });
    }

    let weekly_rate = if front_work > 0 {
        slack_work as f64 / front_work as f64
    } else {
        0.0
    };

    Ok(WeekBill {
        week_start: start_s,
        week_end: we.format("%Y-%m-%d").to_string(),
        week_no: format!("第 {} 周", ws.iso_week().week()),
        is_current_week: false, // week_bill 统一覆盖
        total_income: base_salary + ot_fee,
        base_salary,
        ot_fee,
        slack_cost,
        prev_total: 0.0,
        delta_pct: None,
        work_days: if input.cfg.monitor_activity || input.cfg.monitor_app_usage {
            Some(work_days)
        } else {
            None
        },
        work_hours: work_days as f64 * daily_h + ot_hours_total,
        keys_total,
        clicks_total,
        slack_rate: weekly_rate,
        front_seconds: front_work,
        hardest,
        slackiest,
        days,
    })
}

/// 命令入口：off 封顶 0（未来不可看）；环比 = 上一周 total_income。
/// cfg/hol 由调用方（main.rs）锁内取快照传入，锁外做 DB 查询。
pub fn week_bill(
    cfg: &Config,
    cur_hol: &HolidayCache,
    week_offset: i64,
) -> Result<WeekBill, String> {
    let off = week_offset.max(0);
    let today = Local::now().date_naive();
    let ws = week_start_of(today - Duration::weeks(off));

    let is_wd = is_workday_of(today, cur_hol);
    let mw = monthly_workdays_of(today.year(), today.month(), cfg, cur_hol);
    let today_earned = calc::compute(cfg, is_wd, mw, Local::now()).earned;

    let mut bill = db::with_db(|conn| {
        assemble(
            &WeekInput {
                cfg,
                cur_hol,
                week_start: ws,
                today,
                today_earned,
            },
            conn,
        )
    })?;

    // 上一周：只为取 total_income 做环比（today 不在上一周内，today_earned 不参与）
    let prev = db::with_db(|conn| {
        assemble(
            &WeekInput {
                cfg,
                cur_hol,
                week_start: ws - Duration::days(7),
                today,
                today_earned,
            },
            conn,
        )
    })?;

    bill.is_current_week = off == 0;
    bill.prev_total = prev.total_income;
    bill.delta_pct = delta_pct(bill.total_income, prev.total_income);
    Ok(bill)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn mem_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(db::CREATE_OT_RECORDS).unwrap();
        conn.execute_batch(db::CREATE_ACT_HOURLY).unwrap();
        conn.execute_batch(db::CREATE_APP_USAGE).unwrap();
        conn
    }

    /// 空节假日表 → 一律走周几兜底（周一至五），与 2026 内置表无关，断言确定性
    fn hol() -> HolidayCache {
        HolidayCache {
            year: 2026,
            ..Default::default()
        }
    }

    /// today = 该周周日（休息日）→ 7 天全部按「过去」口径满勤，today_earned 不参与
    fn input<'a>(cfg: &'a Config, hol: &'a HolidayCache, ws: NaiveDate) -> WeekInput<'a> {
        WeekInput {
            cfg,
            cur_hol: hol,
            week_start: ws,
            today: ws + Duration::days(6),
            today_earned: 0.0,
        }
    }

    fn f(v: f64) -> f64 {
        (v * 100.0).round() / 100.0
    }

    #[test]
    fn week_start_alignment() {
        assert_eq!(
            week_start_of(NaiveDate::from_ymd_opt(2026, 9, 9).unwrap()),
            NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()
        );
        assert_eq!(
            week_start_of(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()
        );
        assert_eq!(
            week_start_of(NaiveDate::from_ymd_opt(2026, 9, 13).unwrap()),
            NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()
        );
    }

    #[test]
    fn delta_pct_rules() {
        assert_eq!(delta_pct(110.0, 100.0), Some(10.0));
        assert_eq!(delta_pct(90.0, 100.0), Some(-10.0));
        assert_eq!(delta_pct(5.0, 0.0), None);
        assert_eq!(delta_pct(0.0, 0.0), None);
    }

    #[test]
    fn cross_month_full_salary() {
        // 2026-08-31(周一)..09-06(周日)：8 月分母 21、9 月分母 22（空表周几兜底，晚 8 月无假期）
        let c = Config {
            monthly_salary: 22000.0,
            ..Default::default()
        };
        let conn = mem_conn();
        let bill = assemble(&input(&c, &hol(), NaiveDate::from_ymd_opt(2026, 8, 31).unwrap()), &conn)
            .unwrap();

        assert_eq!(bill.week_start, "2026-08-31");
        assert_eq!(bill.week_end, "2026-09-06");
        assert_eq!(bill.week_no, "第 36 周");
        // 周一 8/31 用 8 月分母
        assert_eq!(f(bill.days[0].salary), f(22000.0 / 21.0));
        // 周二 9/1 起用 9 月分母
        assert_eq!(f(bill.days[1].salary), f(22000.0 / 22.0));
        // 周末 0
        assert_eq!(f(bill.days[5].salary), 0.0);
        assert!(!bill.days[5].is_workday);
        // base = 1×(22000/21) + 4×(22000/22)
        assert_eq!(f(bill.base_salary), f(22000.0 / 21.0 + 4.0 * 22000.0 / 22.0));
        // 总入账 = base + ot（无加班）
        assert_eq!(f(bill.total_income), f(bill.base_salary));
    }

    #[test]
    fn rest_day_salary_zero_but_ot_recorded() {
        let c = Config::default();
        let conn = mem_conn();
        conn.execute(
            "INSERT INTO ot_records (date, lock_time, ot_start, raw_hours, valid_hours, fee, meal, total, source) \
             VALUES ('2026-09-12', '21:00', '19:00', 2.0, 4.0, 10.0, 15.0, 205.0, 0)",
            [],
        )
        .unwrap();
        let bill = assemble(&input(&c, &hol(), NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()), &conn)
            .unwrap();
        // 周六 9/5：休息日无工资，加班费照记
        assert_eq!(f(bill.days[5].salary), 0.0);
        assert!(!bill.days[5].is_workday);
        assert_eq!(f(bill.days[5].ot_total), 205.0);
        assert_eq!(f(bill.ot_fee), 205.0);
        // 休息日不入出勤
        assert_eq!(bill.work_days, Some(0));
    }

    #[test]
    fn act_aggregation() {
        let c = Config::default();
        let conn = mem_conn();
        conn.execute(
            "INSERT INTO act_hourly (date, hour, moves, pixels, `left`, dbl, `right`, wheel, wheel_ticks, mid, xbtn, keys) \
             VALUES ('2026-09-08', 9, 100, 0, 10, 2, 3, 50, 5, 1, 0, 200)",
            [],
        )
        .unwrap();
        let bill = assemble(&input(&c, &hol(), NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()), &conn)
            .unwrap();
        let d = &bill.days[1]; // 周二
        // total_events = 100+10+2+3+50+1+0+200 = 366
        assert_eq!(d.act_events, 366);
        assert_eq!(d.keys, 200);
        // clicks = (10−2×2)+2+3+1+0 = 12
        assert_eq!(d.clicks, 12);
        assert_eq!(bill.keys_total, 200);
        assert_eq!(bill.clicks_total, 12);
        assert!(d.has_record);
        assert_eq!(bill.work_days, Some(1));
    }

    #[test]
    fn slack_rate_and_cost_by_day() {
        // 月薪 22000 / 9 月 22 天 / 日 8h → 时薪 125/h
        let c = Config {
            monthly_salary: 22000.0,
            ..Default::default()
        };
        let conn = mem_conn();
        conn.execute(
            "INSERT INTO app_usage (date, app, seconds) VALUES ('2026-09-08', 'VS Code', 3600)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO app_usage (date, app, seconds) VALUES ('2026-09-08', '网易云音乐', 1200)",
            [],
        )
        .unwrap();
        let bill = assemble(&input(&c, &hol(), NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()), &conn)
            .unwrap();
        let d = &bill.days[1];
        assert_eq!(d.slack_seconds, 1200);
        assert_eq!(f(d.slack_rate), 0.25);
        assert_eq!(f(bill.slack_rate), 0.25);
        // 成本 = 1200/3600 × (22000/22/8) = 0.333×125 = 41.67
        assert_eq!(f(bill.slack_cost), f(1200.0 / 3600.0 * 125.0));
        // 最摸 = 该日
        let s = bill.slackiest.unwrap();
        assert_eq!(s.date, "2026-09-08");
        assert_eq!(s.weekday, "周二");
        // 周摸鱼率分子分母只含工作日（该周仅此一天有记录）
        assert_eq!(bill.front_seconds, 4800);
    }

    #[test]
    fn today_realtime_future_zero() {
        let c = Config {
            monthly_salary: 22000.0,
            ..Default::default()
        };
        let conn = mem_conn();
        let hol = hol();
        let inp = WeekInput {
            cfg: &c,
            cur_hol: &hol,
            week_start: NaiveDate::from_ymd_opt(2026, 9, 7).unwrap(),
            today: NaiveDate::from_ymd_opt(2026, 9, 9).unwrap(), // 周三
            today_earned: 123.0,
        };
        let bill = assemble(&inp, &conn).unwrap();
        // 周一周二满勤
        assert_eq!(f(bill.days[0].salary), f(22000.0 / 22.0));
        // 周三实时
        assert_eq!(f(bill.days[2].salary), 123.0);
        // 周四周五未来 0，且标「未到」由前端按日期判断
        assert_eq!(f(bill.days[3].salary), 0.0);
        assert_eq!(f(bill.days[4].salary), 0.0);
        assert!(bill.days[3].is_workday);
    }

    #[test]
    fn hardest_selection() {
        let c = Config::default();
        let conn = mem_conn();
        for (date, vh, total) in [
            ("2026-09-08", 2.0, 100.0),
            ("2026-09-10", 5.0, 300.0),
            ("2026-09-12", 3.0, 150.0), // 休息日加班
        ] {
            conn.execute(
                "INSERT INTO ot_records (date, lock_time, ot_start, raw_hours, valid_hours, fee, meal, total, source) \
                 VALUES (?1, '21:00', '19:00', 2.0, ?2, 0.0, 0.0, ?3, 0)",
                rusqlite::params![date, vh, total],
            )
            .unwrap();
        }
        let bill = assemble(&input(&c, &hol(), NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()), &conn)
            .unwrap();
        let h = bill.hardest.unwrap();
        assert_eq!(h.date, "2026-09-10");
        assert_eq!(h.ot_hours, 5.0);
        assert_eq!(f(bill.ot_fee), 550.0);
        // work_hours = 出勤工作日(0，无活动记录) × daily_h + Σ valid_hours(10)
        assert_eq!(f(bill.work_hours), 10.0);
    }

    #[test]
    fn monitors_off_work_days_null() {
        let c = Config {
            monitor_activity: false,
            monitor_app_usage: false,
            ..Default::default()
        };
        let conn = mem_conn();
        conn.execute(
            "INSERT INTO act_hourly (date, hour, moves, pixels, `left`, dbl, `right`, wheel, wheel_ticks, mid, xbtn, keys) \
             VALUES ('2026-09-08', 9, 100, 0, 10, 2, 3, 50, 5, 1, 0, 200)",
            [],
        )
        .unwrap();
        let bill = assemble(&input(&c, &hol(), NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()), &conn)
            .unwrap();
        assert_eq!(bill.work_days, None);
    }

    #[test]
    fn empty_week_all_zero() {
        // 无任何监控/加班记录，但历史工作日仍按满勤推算工资（设计口径 1：
        // 库内无打卡工时——满勤不依赖记录），此处断言各颗粒证据为空
        let c = Config {
            monthly_salary: 0.0,
            ..Default::default()
        };
        let conn = mem_conn();
        let bill = assemble(&input(&c, &hol(), NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()), &conn)
            .unwrap();
        assert_eq!(f(bill.total_income), 0.0);
        assert!(bill.days.iter().all(|d| !d.has_record));
        assert!(bill.hardest.is_none());
        assert!(bill.slackiest.is_none());
        assert_eq!(bill.front_seconds, 0);
        assert_eq!(f(bill.slack_cost), 0.0);
    }
}
