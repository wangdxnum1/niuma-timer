//! 周期账单聚合：把一个周期的加班流水、活动计数、应用分类秒折成一张 PeriodBill。
//! 口径见 docs/plans/2026-09-13-week-bill-design.md（口径 1–9）：
//! 满勤工资按天取当月分母、摸鱼成本按天折算、total_income 不减摸鱼成本、
//! 环比同函数聚合上一周期、跨年周按天的年份取内置节假日表且不触发网络。

use std::collections::{BTreeMap, HashMap};

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
    /// 当日四类前台总秒（app_usage 全应用求和；年桶 slack_rate 分母用）
    pub front_seconds: i64,
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

/// 一个周期（周/月/年）的账单。字段名 snake_case 直达前端。
#[derive(Debug, Serialize)]
pub struct PeriodBill {
    pub period_start: String,
    pub period_end: String,
    /// "第 37 周" / "2026 年 9 月" / "2026 年"
    pub period_label: String,
    pub is_current_period: bool,
    pub total_income: f64,
    pub base_salary: f64,
    pub ot_fee: f64,
    pub slack_cost: f64,
    pub prev_total: f64,
    pub delta_pct: Option<f64>,
    pub work_days: Option<i64>,
    pub work_hours: f64,
    /// Σ ot_records.valid_hours（发薪日播报「加班 X.Xh」用）
    pub ot_hours: f64,
    pub keys_total: i64,
    pub clicks_total: i64,
    pub slack_rate: f64,
    pub front_seconds: i64,
    pub hardest: Option<HardestDay>,
    pub slackiest: Option<SlackiestDay>,
    pub buckets: Vec<BucketBill>,
}

/// 汇聚组装所需的全部锁外快照（config/holiday 由调用方取副本，避免锁内做 DB 查询）
pub struct PeriodInput<'a> {
    pub cfg: &'a Config,
    pub cur_hol: &'a HolidayCache,
    /// 账单跨度（决定 period_label 与 B4 的年桶分桶）
    pub span: Span,
    /// 闭区间起点
    pub start: NaiveDate,
    /// 闭区间终点
    pub end: NaiveDate,
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

/// 账单时间跨度（前端以字符串 "week"/"month"/"year" 传入）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Span {
    Week,
    Month,
    Year,
}

/// 解析前端 span 字符串；未知值显式报错，不静默回退（回退会掩盖前端 bug）
pub fn parse_span(s: &str) -> Result<Span, String> {
    match s {
        "week" => Ok(Span::Week),
        "month" => Ok(Span::Month),
        "year" => Ok(Span::Year),
        other => Err(format!("未知账单跨度: {other}")),
    }
}

/// 该月最后一天（下月 1 号 − 1 天，闰年天然正确）
fn last_day_of(year: i32, month: u32) -> u32 {
    let (ny, nm) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    (NaiveDate::from_ymd_opt(ny, nm, 1).unwrap() - Duration::days(1)).day()
}

/// 跨度 + 偏移 → 闭区间 [start, end]（偏移 0 = 当前周期；负值不合法按 0 处理，未来封顶）。
/// Week 分支与原 insights::week_bounds 逐位等价（B5 删除原函数后以此为准）。
pub fn period_bounds(span: Span, offset: i64, today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let off = offset.max(0);
    match span {
        Span::Week => {
            let ws = week_start_of(today - Duration::weeks(off));
            (ws, ws + Duration::days(6))
        }
        Span::Month => {
            let ym = today.year() as i64 * 12 + today.month0() as i64 - off;
            let y = (ym / 12) as i32;
            let m = (ym % 12) as u32 + 1;
            (
                NaiveDate::from_ymd_opt(y, m, 1).unwrap(),
                NaiveDate::from_ymd_opt(y, m, last_day_of(y, m)).unwrap(),
            )
        }
        Span::Year => {
            let y = today.year() - off as i32;
            (
                NaiveDate::from_ymd_opt(y, 1, 1).unwrap(),
                NaiveDate::from_ymd_opt(y, 12, 31).unwrap(),
            )
        }
    }
}

/// 上一周期区间（供环比）：偏移 +1 即语义上的「上一个」
pub fn prev_period_bounds(span: Span, offset: i64, today: NaiveDate) -> (NaiveDate, NaiveDate) {
    period_bounds(span, offset + 1, today)
}

/// 环比百分点；上一周期无基数（≤0）→ None，前端不显示箭头
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
fn monthly_workdays_of(year: i32, month: u32, cfg: &Config, cur: &HolidayCache) -> u32 {
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

/// 周期标签：周 → "第 N 周"；月 → "YYYY 年 M 月"；年 → "YYYY 年"
fn period_label_of(span: Span, start: NaiveDate) -> String {
    match span {
        Span::Week => format!("第 {} 周", start.iso_week().week()),
        Span::Month => format!("{} 年 {} 月", start.year(), start.month()),
        Span::Year => format!("{} 年", start.year()),
    }
}

/// 周期聚合（conn 由调用方给：正式走 with_db，单测走 in-memory；跨度为周/月/年）
pub fn assemble(input: &PeriodInput, conn: &Connection) -> rusqlite::Result<PeriodBill> {
    let ws = input.start;
    let we = input.end;
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
    let mut days: Vec<DayBill> = Vec::new();
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

    let mut d = input.start;
    while d <= input.end {
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

        let (ot_total, ot_hours) = ot_map.get(&key).map(|t| (t.0, t.1)).unwrap_or((0.0, 0.0));
        let (act_events, keys, clicks) = act_map.get(&key).copied().unwrap_or((0, 0, 0));
        let (front, slack) = app_map.get(&key).copied().unwrap_or((0, 0));
        let has_record = act_events > 0 || front > 0;

        // 当日时薪 = 满勤 ÷ 当日工时（固定时薪；今天实时 earned 只影响 salary 字段）
        let day_rate = if daily_h > 0.0 {
            full_salary / daily_h
        } else {
            0.0
        };
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
            front_seconds: front,
            has_record,
        });
        d += Duration::days(1);
    }

    let weekly_rate = if front_work > 0 {
        slack_work as f64 / front_work as f64
    } else {
        0.0
    };

    Ok(PeriodBill {
        period_start: start_s,
        period_end: we.format("%Y-%m-%d").to_string(),
        period_label: period_label_of(input.span, ws),
        is_current_period: false, // period_bill 统一覆盖
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
        ot_hours: ot_hours_total,
        buckets: bucketize(&days, input.span),
    })
}

/// 命令入口：off 封顶 0（未来不可看）；环比 = 上一周期 total_income。
/// cfg/hol 由调用方（main.rs）锁内取快照传入，锁外做 DB 查询。
pub fn period_bill(
    cfg: &Config,
    cur_hol: &HolidayCache,
    span: Span,
    offset: i64,
) -> Result<PeriodBill, String> {
    let off = offset.max(0);
    let today = Local::now().date_naive();
    let (start, end) = period_bounds(span, off, today);

    let is_wd = is_workday_of(today, cur_hol);
    let mw = monthly_workdays_of(today.year(), today.month(), cfg, cur_hol);
    let today_earned = calc::compute(cfg, is_wd, mw, Local::now()).earned;

    let mut bill = db::with_db(|conn| {
        assemble(
            &PeriodInput {
                cfg,
                cur_hol,
                span,
                start,
                end,
                today,
                today_earned,
            },
            conn,
        )
    })?;

    // 上一周期：只为取 total_income 做环比（严格早于当前周期，today 不在其中，today_earned 不参与）
    let (prev_start, prev_end) = prev_period_bounds(span, off, today);
    let prev = db::with_db(|conn| {
        assemble(
            &PeriodInput {
                cfg,
                cur_hol,
                span,
                start: prev_start,
                end: prev_end,
                today,
                today_earned,
            },
            conn,
        )
    })?;

    bill.is_current_period = off == 0;
    bill.prev_total = prev.total_income;
    bill.delta_pct = delta_pct(bill.total_income, prev.total_income);
    Ok(bill)
}

/// 分桶账单（前端唯一渲染颗粒）：周/月 → 天桶；年 → 12 个月桶。
#[derive(Debug, Serialize)]
pub struct BucketBill {
    /// 天桶 = 具体日期 "2026-09-07"；月桶 = None（未来日判定与日号切片在前端按空处理）
    pub date: Option<String>,
    /// 天桶「周一」；月桶「9 月」
    pub label: String,
    /// 天桶 = 法定口径；月桶 = None（不参与「休/未到」判定）
    pub is_workday: Option<bool>,
    pub salary: f64,
    pub ot_total: f64,
    pub slack_seconds: i64,
    pub act_events: i64,
    pub keys: i64,
    pub clicks: i64,
    /// 天桶 = 当日比率；月桶 = Σ摸鱼秒 ÷ Σ(工作日且有记录的前台秒)（求和后再相除，非比率平均）
    pub slack_rate: f64,
    pub has_record: bool,
}

/// 天桶透传：周/月视图的渲染颗粒与逐日账单同构
fn day_to_bucket(d: &DayBill) -> BucketBill {
    BucketBill {
        date: Some(d.date.clone()),
        label: d.weekday.clone(),
        is_workday: Some(d.is_workday),
        salary: d.salary,
        ot_total: d.ot_total,
        slack_seconds: d.slack_seconds,
        act_events: d.act_events,
        keys: d.keys,
        clicks: d.clicks,
        slack_rate: d.slack_rate,
        has_record: d.has_record,
    }
}

/// 年桶：同月各天合并。slack_rate 分子分母均按 is_workday && has_record 求和后再相除
/// （口径与周聚合一致，不是各天比率平均）；front > 0 才算比率，空桶 0.0
fn month_bucket(label: String, ds: Vec<&DayBill>) -> BucketBill {
    let mut b = BucketBill {
        date: None,
        label,
        is_workday: None,
        salary: 0.0,
        ot_total: 0.0,
        slack_seconds: 0,
        act_events: 0,
        keys: 0,
        clicks: 0,
        slack_rate: 0.0,
        has_record: false,
    };
    let mut front = 0i64;
    for d in ds {
        b.salary += d.salary;
        b.ot_total += d.ot_total;
        b.act_events += d.act_events;
        b.keys += d.keys;
        b.clicks += d.clicks;
        b.has_record |= d.has_record;
        if d.is_workday && d.has_record {
            b.slack_seconds += d.slack_seconds;
            front += d.front_seconds;
        }
    }
    b.slack_rate = if front > 0 {
        b.slack_seconds as f64 / front as f64
    } else {
        0.0
    };
    b
}

/// 把逐日账单按跨度折成分桶：周/月 → 天桶透传；年 → 12 个月桶（后端算好，前端单一路径渲染）
pub fn bucketize(days: &[DayBill], span: Span) -> Vec<BucketBill> {
    match span {
        Span::Year => {
            // 12 恒项：无记录的月份也要出桶（前端画 12 根柱，缺月 = 空柱）
            let mut by_month: BTreeMap<u32, Vec<&DayBill>> = BTreeMap::new();
            for d in days {
                if let Some(m) = d.date.get(5..7).and_then(|s| s.parse::<u32>().ok()) {
                    by_month.entry(m).or_default().push(d);
                }
            }
            (1u32..=12)
                .map(|m| month_bucket(format!("{m} 月"), by_month.remove(&m).unwrap_or_default()))
                .collect()
        }
        _ => days.iter().map(day_to_bucket).collect(),
    }
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
    fn input<'a>(cfg: &'a Config, hol: &'a HolidayCache, ws: NaiveDate) -> PeriodInput<'a> {
        PeriodInput {
            cfg,
            cur_hol: hol,
            span: Span::Week,
            start: ws,
            end: ws + Duration::days(6),
            today: ws + Duration::days(6),
            today_earned: 0.0,
        }
    }

    fn f(v: f64) -> f64 {
        (v * 100.0).round() / 100.0
    }

    /// 2026-09 整月播种：22 个工作日各插一条 act_hourly（无加班、无 app 记录）
    fn seed_2026_09_full_month(conn: &Connection) {
        let empty_hol = hol();
        let mut d = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        while d.month() == 9 {
            if is_workday_of(d, &empty_hol) {
                conn.execute(
                    "INSERT INTO act_hourly (date, hour, moves, pixels, `left`, dbl, `right`, wheel, wheel_ticks, mid, xbtn, keys) \
                     VALUES (?1, 9, 100, 0, 10, 2, 3, 50, 5, 1, 0, 200)",
                    rusqlite::params![d.to_string()],
                )
                .unwrap();
            }
            d += Duration::days(1);
        }
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
        let bill = assemble(
            &input(&c, &hol(), NaiveDate::from_ymd_opt(2026, 8, 31).unwrap()),
            &conn,
        )
        .unwrap();

        assert_eq!(bill.period_start, "2026-08-31");
        assert_eq!(bill.period_end, "2026-09-06");
        assert_eq!(bill.period_label, "第 36 周");
        // 周一 8/31 用 8 月分母
        assert_eq!(f(bill.buckets[0].salary), f(22000.0 / 21.0));
        // 周二 9/1 起用 9 月分母
        assert_eq!(f(bill.buckets[1].salary), f(22000.0 / 22.0));
        // 周末 0
        assert_eq!(f(bill.buckets[5].salary), 0.0);
        assert_eq!(bill.buckets[5].is_workday, Some(false));
        // base = 1×(22000/21) + 4×(22000/22)
        assert_eq!(
            f(bill.base_salary),
            f(22000.0 / 21.0 + 4.0 * 22000.0 / 22.0)
        );
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
        let bill = assemble(
            &input(&c, &hol(), NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            &conn,
        )
        .unwrap();
        // 周六 9/5：休息日无工资，加班费照记
        assert_eq!(f(bill.buckets[5].salary), 0.0);
        assert_eq!(bill.buckets[5].is_workday, Some(false));
        assert_eq!(f(bill.buckets[5].ot_total), 205.0);
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
        let bill = assemble(
            &input(&c, &hol(), NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            &conn,
        )
        .unwrap();
        let d = &bill.buckets[1]; // 周二
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
        let bill = assemble(
            &input(&c, &hol(), NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            &conn,
        )
        .unwrap();
        let d = &bill.buckets[1];
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
        let inp = PeriodInput {
            cfg: &c,
            cur_hol: &hol,
            span: Span::Week,
            start: NaiveDate::from_ymd_opt(2026, 9, 7).unwrap(),
            end: NaiveDate::from_ymd_opt(2026, 9, 7).unwrap() + Duration::days(6),
            today: NaiveDate::from_ymd_opt(2026, 9, 9).unwrap(), // 周三
            today_earned: 123.0,
        };
        let bill = assemble(&inp, &conn).unwrap();
        // 周一周二满勤
        assert_eq!(f(bill.buckets[0].salary), f(22000.0 / 22.0));
        // 周三实时
        assert_eq!(f(bill.buckets[2].salary), 123.0);
        // 周四周五未来 0，且标「未到」由前端按日期判断
        assert_eq!(f(bill.buckets[3].salary), 0.0);
        assert_eq!(f(bill.buckets[4].salary), 0.0);
        assert_eq!(bill.buckets[3].is_workday, Some(true));
    }

    #[test]
    fn month_assemble_spans_full_month() {
        // 2026-09 整月；today 放 10-01 避开实时分支（本月第一天，区间全为过去）
        let conn = mem_conn();
        seed_2026_09_full_month(&conn); // 沿用测试区播种范式：22000 月薪、9 月 22 个工作日有打卡
        let c = Config {
            monthly_salary: 22000.0,
            ..Default::default()
        };
        let h = hol();
        let pi = PeriodInput {
            cfg: &c,
            cur_hol: &h,
            span: Span::Month,
            start: NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
            end: NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
            today: NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            today_earned: 0.0,
        };
        let bill = assemble(&pi, &conn).unwrap();
        assert_eq!(bill.period_start, "2026-09-01");
        assert_eq!(bill.period_end, "2026-09-30");
        assert_eq!(bill.period_label, "2026 年 9 月");
        assert_eq!(bill.buckets.len(), 30);
        assert_eq!(f(bill.base_salary), 22000.0);
        assert_eq!(bill.work_days, Some(22)); // 2026 年 9 月工作日 22 天（空表周几兜底口径）
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
        let bill = assemble(
            &input(&c, &hol(), NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            &conn,
        )
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
        let bill = assemble(
            &input(&c, &hol(), NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            &conn,
        )
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
        let bill = assemble(
            &input(&c, &hol(), NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()),
            &conn,
        )
        .unwrap();
        assert_eq!(f(bill.total_income), 0.0);
        assert!(bill.buckets.iter().all(|d| !d.has_record));
        assert!(bill.hardest.is_none());
        assert!(bill.slackiest.is_none());
        assert_eq!(bill.front_seconds, 0);
        assert_eq!(f(bill.slack_cost), 0.0);
    }

    #[test]
    fn period_bounds_week_matches_old_week_bounds() {
        // today = 2026-09-23（周三），偏移 2 → 2026-09-07（周一）..2026-09-13（周日）
        let today = NaiveDate::from_ymd_opt(2026, 9, 23).unwrap();
        let (s, e) = period_bounds(Span::Week, 2, today);
        assert_eq!(s, NaiveDate::from_ymd_opt(2026, 9, 7).unwrap());
        assert_eq!(e, NaiveDate::from_ymd_opt(2026, 9, 13).unwrap());
        // 负偏移封顶本期（未来不可看）
        assert_eq!(
            period_bounds(Span::Week, -5, today),
            period_bounds(Span::Week, 0, today)
        );
        // 任何周区间恒为 7 天
        let (s, e) = period_bounds(Span::Week, 3, today);
        assert_eq!((e - s).num_days() + 1, 7);
    }

    #[test]
    fn period_bounds_month_year() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 23).unwrap();
        // 9 月：1 号到 30 号
        let (s, e) = period_bounds(Span::Month, 0, today);
        assert_eq!(
            (s, e),
            (
                NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
            )
        );
        // 跨年：1 月的上一期是去年 12 月
        let jan = NaiveDate::from_ymd_opt(2026, 1, 10).unwrap();
        assert_eq!(
            prev_period_bounds(Span::Month, 0, jan),
            (
                NaiveDate::from_ymd_opt(2025, 12, 1).unwrap(),
                NaiveDate::from_ymd_opt(2025, 12, 31).unwrap(),
            )
        );
        // 闰年：2024 年 2 月有 29 天
        let (s, e) = period_bounds(
            Span::Month,
            0,
            NaiveDate::from_ymd_opt(2024, 2, 10).unwrap(),
        );
        assert_eq!(s, NaiveDate::from_ymd_opt(2024, 2, 1).unwrap());
        assert_eq!(e.day(), 29);
        // 年区间
        let (s, e) = period_bounds(Span::Year, 1, today);
        assert_eq!(
            (s, e),
            (
                NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
                NaiveDate::from_ymd_opt(2025, 12, 31).unwrap(),
            )
        );
    }

    #[test]
    fn prev_period_bounds_is_offset_plus_one() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 23).unwrap();
        for span in [Span::Week, Span::Month, Span::Year] {
            for off in 0..3 {
                assert_eq!(
                    prev_period_bounds(span, off, today),
                    period_bounds(span, off + 1, today)
                );
            }
        }
    }

    #[test]
    fn parse_span_rejects_unknown() {
        assert_eq!(parse_span("week"), Ok(Span::Week));
        assert_eq!(parse_span("month"), Ok(Span::Month));
        assert_eq!(parse_span("year"), Ok(Span::Year));
        assert!(parse_span("decade").is_err());
    }

    /// 手构 DayBill（B4 测试专用；全字段显式，slack_rate 由 slack/front 求出）
    fn dbill(
        date: &str,
        weekday: &str,
        is_workday: bool,
        has_record: bool,
        salary: f64,
        slack: i64,
        front: i64,
    ) -> DayBill {
        DayBill {
            date: date.to_string(),
            weekday: weekday.to_string(),
            is_workday,
            salary,
            ot_total: 0.0,
            slack_seconds: slack,
            act_events: 0,
            keys: 0,
            clicks: 0,
            slack_rate: if front > 0 {
                slack as f64 / front as f64
            } else {
                0.0
            },
            front_seconds: front,
            has_record,
        }
    }

    #[test]
    fn year_buckets_aggregate_with_summed_front_rate() {
        // 2026 年：1 月两天（200/1000 与 1800/3000），2 月一天（0/2000），其余月无记录
        let days = vec![
            dbill("2026-01-05", "周一", true, true, 1000.0, 200, 1000),
            dbill("2026-01-06", "周二", true, true, 1000.0, 1800, 3000),
            dbill("2026-02-09", "周一", true, true, 1000.0, 0, 2000),
        ];
        let b = bucketize(&days, Span::Year);
        assert_eq!(b.len(), 12); // 12 恒项：缺月 = 空桶
                                 // 年桶摸鱼率 = Σ摸鱼秒 ÷ Σ前台秒（200+1800)/(1000+3000) = 0.5，不是各天比率平均 (0.2+0.6)/2 = 0.4
        assert_eq!(f(b[0].slack_rate), 0.5);
        assert_eq!(b[0].salary, 2000.0);
        assert_eq!(b[1].slack_rate, 0.0); // 0/2000
        assert_eq!(b[1].has_record, true);
        assert_eq!(b[2].label, "3 月");
        assert_eq!(b[2].has_record, false); // 空桶
        assert_eq!(b[11].label, "12 月");
        assert_eq!(b[0].date, None);
        assert_eq!(b[0].is_workday, None);
    }

    #[test]
    fn week_buckets_passthrough_with_weekday_labels() {
        let days = vec![
            dbill("2026-09-07", "周一", true, true, 1000.0, 100, 1000),
            dbill("2026-09-13", "周日", false, false, 0.0, 0, 0),
        ];
        let b = bucketize(&days, Span::Week);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].date.as_deref(), Some("2026-09-07"));
        assert_eq!(b[0].label, "周一");
        assert_eq!(b[0].is_workday, Some(true));
        assert_eq!(b[1].is_workday, Some(false));
        assert_eq!(f(b[0].slack_rate), 0.1);
    }

    #[test]
    fn month_buckets_keep_daily_grain() {
        let days: Vec<DayBill> = (1..=30)
            .map(|i| {
                dbill(
                    &format!("2026-09-{:02}", i),
                    "周几",
                    i % 7 != 6 && i % 7 != 0,
                    i < 23,
                    1000.0,
                    10,
                    100,
                )
            })
            .collect();
        let b = bucketize(&days, Span::Month);
        assert_eq!(b.len(), 30); // 月视图保持天颗粒
        assert_eq!(b[0].date.as_deref(), Some("2026-09-01"));
    }
}
