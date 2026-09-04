//! 加班记录：数据结构、费用计算、SQLite 持久化（ot_records 表）。

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime, TimeZone, Timelike, Weekday};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::config::Config;

/// 记录来源：自动（锁屏离开时生成）。可被后续自动记录覆盖。
pub const SOURCE_AUTO: i32 = 0;
/// 记录来源：手动录入。优先级最高，自动路径**不得**覆盖（见 `upsert_auto`）。
pub const SOURCE_MANUAL: i32 = 1;

/// 单日加班记录
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OvertimeRecord {
    /// 日期 "2026-08-19"
    pub date: String,
    /// 下班时间（最后一次锁屏离开）"20:30"
    pub lock_time: String,
    /// 加班起算时间 "18:00"
    pub ot_start: String,
    /// 原始加班时长（小时）
    pub raw_hours: f64,
    /// 有效加班时长（向下取 0.5h，不足 1h 为 0）
    pub valid_hours: f64,
    /// 加班费（元）
    pub fee: f64,
    /// 饭补（元）
    pub meal: f64,
    /// 当日合计（元）
    pub total: f64,
    /// 来源：`SOURCE_AUTO`(0) 自动 / `SOURCE_MANUAL`(1) 手动。
    /// 自动 upsert 只覆盖自动记录，手改过的记录不会被当晚的锁屏数据顶掉。
    pub source: i32,
}

/// 月度加班记录集合（持久化结构体，只存原始 records）
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MonthlyOvertime {
    pub records: Vec<OvertimeRecord>,
}

/// 月度加班汇总视图（含预计算汇总字段，序列化返回前端）
#[derive(Clone, Debug, Serialize)]
pub struct MonthlyOvertimeView {
    pub records: Vec<OvertimeRecord>,
    pub total_hours: f64,
    pub total_fee: f64,
    pub total_meal: f64,
    pub total_all: f64,
    pub days: usize,
}

impl MonthlyOvertime {
    pub fn total_hours(&self) -> f64 {
        self.records.iter().map(|r| r.valid_hours).sum()
    }
    pub fn total_fee(&self) -> f64 {
        self.records.iter().map(|r| r.fee).sum()
    }
    pub fn total_meal(&self) -> f64 {
        self.records.iter().map(|r| r.meal).sum()
    }
    pub fn total_all(&self) -> f64 {
        self.records.iter().map(|r| r.total).sum()
    }
    pub fn days(&self) -> usize {
        self.records.len()
    }

    /// 转为前端响应视图（records + 预计算汇总）
    pub fn to_view(&self) -> MonthlyOvertimeView {
        MonthlyOvertimeView {
            total_hours: self.total_hours(),
            total_fee: self.total_fee(),
            total_meal: self.total_meal(),
            total_all: self.total_all(),
            days: self.days(),
            records: self.records.clone(),
        }
    }
}

/// 方案一：有效时长计算
/// - 不足 1 小时 → 0（无效）
/// - >= 1 小时 → 向下取 0.5 小时
///   例: 1.3→1.0, 1.6→1.5, 2.0→2.0
pub fn calc_valid_hours(raw_hours: f64) -> f64 {
    if raw_hours < 1.0 {
        return 0.0;
    }
    (raw_hours * 2.0).floor() / 2.0
}

/// "HH:MM" → 当天分钟数
fn to_min(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() == 2 {
        let h = parts[0].parse::<f64>().ok()?;
        let m = parts[1].parse::<f64>().ok()?;
        Some(h * 60.0 + m)
    } else {
        None
    }
}

/// 分钟数 → "HH:MM"
fn format_hm(min: f64) -> String {
    let h = (min / 60.0).floor() as i32;
    let m = (min % 60.0).round() as i32;
    format!("{:02}:{:02}", h, m)
}

/// 根据锁屏时间（=下班离开时刻）和配置计算单日加班记录（自动锁屏路径）
/// 返回 None 表示无有效加班（下班早于起算时间、不足 1 小时等）
pub fn calc_record(
    date: NaiveDate,
    lock_time: DateTime<Local>,
    cfg: &Config,
) -> Option<OvertimeRecord> {
    // 加班起算时间：overtime_start 有值则用，否则用 pm_end
    let ot_start_str = cfg.overtime_start.as_deref().unwrap_or(&cfg.pm_end);
    compute_record(date, lock_time, ot_start_str, cfg)
}

/// 核心计算：给定明确的加班起算时间字符串，计算单日记录。
/// 抽出来供自动锁屏路径与手动录入路径共用，保证规则一致。
fn compute_record(
    date: NaiveDate,
    lock_time: DateTime<Local>,
    ot_start_str: &str,
    cfg: &Config,
) -> Option<OvertimeRecord> {
    // 周末加班受配置开关约束：周六/周日且未开启 weekend_overtime 时不计入加班。
    // 按自然周几判断；调休补班日暂按周末处理（加班计算未接入 holiday 日类型）。
    if !cfg.weekend_overtime && matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
        return None;
    }

    let ot_start_min = to_min(ot_start_str)?;

    let lock_min = lock_time.hour() as f64 * 60.0
        + lock_time.minute() as f64
        + lock_time.second() as f64 / 60.0;

    // 下班（锁屏离开）必须晚于起算时间
    if lock_min <= ot_start_min {
        return None;
    }

    let raw_hours = (lock_min - ot_start_min) / 60.0;
    let valid_hours = calc_valid_hours(raw_hours);

    if valid_hours < 1.0 {
        return None; // 不足 1 小时
    }

    let fee = valid_hours * cfg.overtime_rate;
    let meal = if cfg.overtime_meal_enabled {
        cfg.overtime_meal
    } else {
        0.0
    };
    let total = fee + meal;

    Some(OvertimeRecord {
        date: date.format("%Y-%m-%d").to_string(),
        lock_time: lock_time.format("%H:%M").to_string(),
        ot_start: format_hm(ot_start_min),
        raw_hours: (raw_hours * 10.0).round() / 10.0, // 保留 1 位小数
        valid_hours,
        fee,
        meal,
        total,
        source: SOURCE_AUTO, // 计算出来的记录一律是自动来源，手改由 save_manual 改写
    })
}

/// 手动录入请求：前端提交日期 + 下班时间（+ 可选起算时间覆盖）
#[derive(Clone, Debug, Deserialize)]
pub struct ManualOvertimeInput {
    /// 日期 "2026-08-19"
    pub date: String,
    /// 下班时间（最后一次锁屏离开）"20:30"
    pub lock_time: String,
    /// 可选：覆盖加班起算时间 "HH:MM"；None 用配置默认值
    pub ot_start: Option<String>,
}

/// "YYYY-MM-DD" + "HH:MM" → 当天本地 DateTime
fn parse_lock_datetime(date: &str, time: &str) -> Option<DateTime<Local>> {
    let d = NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    let parts: Vec<&str> = time.split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let h: u32 = parts[0].parse().ok()?;
    let m: u32 = parts[1].parse().ok()?;
    let nt = NaiveTime::from_hms_opt(h, m, 0)?;
    let ndt = d.and_time(nt);
    Local.from_local_datetime(&ndt).single()
}

/// 判断给定日期字符串是否属于当前月份
pub fn is_current_month(date: &str) -> bool {
    let d = match NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => return false,
    };
    let now = Local::now();
    d.year() == now.year() && d.month() == now.month()
}

/// 手动添加/修改某天加班记录（按日期 upsert）。
/// 仅允许当前月份；返回生成的记录或错误信息。
pub fn save_manual(
    input: ManualOvertimeInput,
    cfg: &Config,
) -> Result<OvertimeRecord, String> {
    let date = NaiveDate::parse_from_str(&input.date, "%Y-%m-%d")
        .map_err(|_| "日期格式错误".to_string())?;
    if !is_current_month(&input.date) {
        return Err("只能添加或修改当月的数据".to_string());
    }
    // 周末加班受开关约束：手动录入周末且未开启开关时给出明确提示
    if !cfg.weekend_overtime && matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
        return Err("周末加班功能未开启，请在设置中开启「周末加班」".to_string());
    }
    let lock_dt = parse_lock_datetime(&input.date, &input.lock_time)
        .ok_or_else(|| "下班时间格式错误，应为 HH:MM".to_string())?;
    // 起算时间：手动覆盖 > 配置 overtime_start > pm_end
    let ot_start_str = input
        .ot_start
        .as_deref()
        .or(cfg.overtime_start.as_deref())
        .unwrap_or(&cfg.pm_end);
    // 预校验，给出明确错误（compute_record 返回 None 时无法区分原因）
    let ot_start_min = to_min(ot_start_str)
        .ok_or_else(|| "加班起算时间格式错误".to_string())?;
    let lock_min = lock_dt.hour() as f64 * 60.0
        + lock_dt.minute() as f64
        + lock_dt.second() as f64 / 60.0;
    if lock_min <= ot_start_min {
        return Err(format!(
            "下班时间 {} 需晚于加班起算时间 {}",
            input.lock_time, ot_start_str
        ));
    }
    let raw_hours = (lock_min - ot_start_min) / 60.0;
    if calc_valid_hours(raw_hours) < 1.0 {
        return Err("加班时长不足 1 小时，无法生成有效记录".to_string());
    }
    let rec = compute_record(date, lock_dt, ot_start_str, cfg)
        .ok_or_else(|| "无法生成加班记录".to_string())?;
    // 标为手动来源：此后自动锁屏记录不再覆盖这一天（见 upsert_auto）
    upsert_manual(rec.clone());
    Ok(rec)
}

/// 手动删除某天加班记录。仅允许当前月份。
pub fn delete_manual(date: &str) -> Result<(), String> {
    if !is_current_month(date) {
        return Err("只能删除当月的数据".to_string());
    }
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|_| "日期格式错误".to_string())?;
    let g = crate::db::conn().lock().unwrap();
    let n = g
        .execute("DELETE FROM ot_records WHERE date = ?1", params![date])
        .map_err(|e| format!("删除失败: {e}"))?;
    if n == 0 {
        return Err("未找到该日期的加班记录".to_string());
    }
    Ok(())
}

// ---- 持久化（SQLite） ----

/// 写入一条加班记录到指定连接（date 主键，同日按 `force` 决定是否覆盖）。
///
/// 只写 `?` 参数化的 UPSERT，**不用 `INSERT OR REPLACE`**：后者的语义是
/// 「冲突时先 DELETE 再 INSERT」，无法附加条件，做不到「手改的记录不让自动覆盖」。
///
/// - `force = true`（手动路径）：无条件覆盖同名日期行；
/// - `force = false`（自动路径）：`WHERE ot_records.source = 0` 只在既有行**也是自动
///   来源**时才覆盖，手动记录（`source = 1`）原样保留，一次锁屏不会顶掉用户的手改值。
///
/// 抽出「接受 `&Connection`」这一层是为了让单测能在 in-memory 库上直接验证覆盖语义，
/// 不必碰真实的 `%APPDATA%/niuma-timer/niuma.db`。
fn upsert_into(g: &Connection, record: &OvertimeRecord, force: bool) -> rusqlite::Result<()> {
    g.execute(
        "INSERT INTO ot_records \
         (date, lock_time, ot_start, raw_hours, valid_hours, fee, meal, total, source) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
         ON CONFLICT(date) DO UPDATE SET \
           lock_time   = excluded.lock_time, \
           ot_start    = excluded.ot_start, \
           raw_hours   = excluded.raw_hours, \
           valid_hours = excluded.valid_hours, \
           fee         = excluded.fee, \
           meal        = excluded.meal, \
           total       = excluded.total, \
           source      = excluded.source \
         WHERE ?10 <> 0 OR ot_records.source = 0",
        params![
            record.date,
            record.lock_time,
            record.ot_start,
            record.raw_hours,
            record.valid_hours,
            record.fee,
            record.meal,
            record.total,
            record.source,
            force as i32
        ],
    )?;
    Ok(())
}

/// **自动**路径 upsert（锁屏离开时调用）。
///
/// 关键保护：只覆盖同为自动来源的记录。用户手动改过某天（`source = 1`）之后，
/// 当晚再次锁屏产生的自动数据不会把手改值顶掉——改之前这个动作是静默发生的。
pub fn upsert_auto(record: OvertimeRecord) {
    let mut rec = record;
    rec.source = SOURCE_AUTO; // 来源由路径决定，不信任调用方传入的值
    let g = crate::db::conn().lock().unwrap();
    let _ = upsert_into(&g, &rec, false);
}

/// **手动**路径 upsert（加班明细页增删改）。手改是用户明确意图，优先级最高，无条件覆盖。
pub fn upsert_manual(record: OvertimeRecord) {
    let mut rec = record;
    rec.source = SOURCE_MANUAL;
    let g = crate::db::conn().lock().unwrap();
    let _ = upsert_into(&g, &rec, true);
}

/// 获取指定月份的加班记录（SQL 按日期前缀过滤，历史月与当月同表，无需归档）。
pub fn get_month(year: i32, month: u32) -> MonthlyOvertime {
    let prefix = format!("{:04}-{:02}-", year, month);
    let g = crate::db::conn().lock().unwrap();
    let mut stmt = match g.prepare(
        "SELECT date, lock_time, ot_start, raw_hours, valid_hours, fee, meal, total, source \
         FROM ot_records WHERE date LIKE ?1 ORDER BY date",
    ) {
        Ok(s) => s,
        Err(_) => return MonthlyOvertime::default(),
    };
    let rows = stmt.query_map(params![format!("{prefix}%")], |r| {
        Ok(OvertimeRecord {
            date: r.get(0)?,
            lock_time: r.get(1)?,
            ot_start: r.get(2)?,
            raw_hours: r.get(3)?,
            valid_hours: r.get(4)?,
            fee: r.get(5)?,
            meal: r.get(6)?,
            total: r.get(7)?,
            source: r.get(8)?,
        })
    });
    match rows {
        Ok(iter) => MonthlyOvertime {
            records: iter.filter_map(|r| r.ok()).collect(),
        },
        Err(_) => MonthlyOvertime::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, NaiveDate, NaiveTime, TimeZone};

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    /// 构造本地下班时刻（锁屏离开时刻）
    fn lock_dt(y: i32, m: u32, d: u32, hh: u32, mm: u32) -> DateTime<Local> {
        let date = NaiveDate::from_ymd_opt(y, m, d).unwrap();
        Local
            .from_local_datetime(&date.and_time(NaiveTime::from_hms_opt(hh, mm, 0).unwrap()))
            .single()
            .unwrap()
    }

    #[test]
    fn calc_valid_hours_rounds_down() {
        // 不足 1h → 0；>=1h 向下取 0.5h
        assert!(approx(calc_valid_hours(0.5), 0.0));
        assert!(approx(calc_valid_hours(0.99), 0.0));
        assert!(approx(calc_valid_hours(1.0), 1.0));
        assert!(approx(calc_valid_hours(1.3), 1.0));
        assert!(approx(calc_valid_hours(1.6), 1.5));
        assert!(approx(calc_valid_hours(2.0), 2.0));
        assert!(approx(calc_valid_hours(2.4), 2.0));
        assert!(approx(calc_valid_hours(2.6), 2.5));
        assert!(approx(calc_valid_hours(3.0), 3.0));
        assert!(approx(calc_valid_hours(8.9), 8.5));
    }

    #[test]
    fn to_min_option() {
        assert!(approx(to_min("18:00").expect("some"), 1080.0));
        assert!(approx(to_min("18:30").expect("some"), 1110.0));
        assert!(to_min("abc").is_none());
        assert!(to_min("18:xx").is_none());
        assert!(to_min("18").is_none());
    }

    #[test]
    fn format_hm_basic() {
        assert_eq!(format_hm(540.0), "09:00");
        assert_eq!(format_hm(1110.0), "18:30");
        assert_eq!(format_hm(60.0), "01:00");
        assert_eq!(format_hm(0.0), "00:00");
    }

    #[test]
    fn parse_lock_datetime_basic() {
        assert!(parse_lock_datetime("2026-08-19", "20:30").is_some());
        assert!(parse_lock_datetime("2026-08-19", "25:00").is_none());
        assert!(parse_lock_datetime("2026-13-01", "20:30").is_none());
        assert!(parse_lock_datetime("2026-08-19", "20").is_none());
    }

    #[test]
    fn compute_record_weekday_valid() {
        let cfg = Config::default();
        let date = NaiveDate::from_ymd_opt(2026, 8, 19).unwrap(); // 周三
        let r = compute_record(date, lock_dt(2026, 8, 19, 20, 30), "18:00", &cfg)
            .expect("应生成记录");
        assert_eq!(r.date, "2026-08-19");
        assert_eq!(r.lock_time, "20:30");
        assert_eq!(r.ot_start, "18:00");
        assert!(approx(r.raw_hours, 2.5));
        assert!(approx(r.valid_hours, 2.5));
        assert!(approx(r.fee, 50.0)); // 2.5 * 20
        assert!(approx(r.meal, 20.0)); // 默认开启饭补
        assert!(approx(r.total, 70.0));
    }

    #[test]
    fn compute_record_short_overtime_none() {
        let cfg = Config::default();
        let date = NaiveDate::from_ymd_opt(2026, 8, 19).unwrap();
        // 18:30 离开，加班 0.5h < 1h → 无有效记录
        assert!(compute_record(date, lock_dt(2026, 8, 19, 18, 30), "18:00", &cfg).is_none());
    }

    #[test]
    fn compute_record_weekend_disabled_none() {
        let cfg = Config::default(); // weekend_overtime=false
        let date = NaiveDate::from_ymd_opt(2026, 8, 22).unwrap(); // 周六
        assert!(compute_record(date, lock_dt(2026, 8, 22, 22, 0), "18:00", &cfg).is_none());
    }

    #[test]
    fn compute_record_weekend_enabled_some() {
        let mut cfg = Config::default();
        cfg.weekend_overtime = true;
        let date = NaiveDate::from_ymd_opt(2026, 8, 22).unwrap(); // 周六
        let r = compute_record(date, lock_dt(2026, 8, 22, 22, 0), "18:00", &cfg)
            .expect("周末开启应生成");
        assert!(approx(r.valid_hours, 4.0));
        assert!(approx(r.fee, 80.0));
        assert!(approx(r.total, 100.0));
    }

    #[test]
    fn compute_record_custom_start() {
        let cfg = Config::default();
        let date = NaiveDate::from_ymd_opt(2026, 8, 19).unwrap();
        // 自定义起算 19:00，20:30 离开 → 1.5h
        let r = compute_record(date, lock_dt(2026, 8, 19, 20, 30), "19:00", &cfg).expect("应生成");
        assert_eq!(r.ot_start, "19:00");
        assert!(approx(r.raw_hours, 1.5));
        assert!(approx(r.valid_hours, 1.5));
        assert!(approx(r.fee, 30.0));
        assert!(approx(r.total, 50.0));
    }

    #[test]
    fn calc_record_defaults_to_pm_end() {
        let cfg = Config::default(); // overtime_start=None → 用 pm_end 18:00
        let date = NaiveDate::from_ymd_opt(2026, 8, 19).unwrap();
        let r = calc_record(date, lock_dt(2026, 8, 19, 20, 0), &cfg).expect("应生成");
        assert_eq!(r.ot_start, "18:00");
        assert!(approx(r.valid_hours, 2.0));
    }

    // ---- 覆盖语义：手动记录必须保住，不能被自动锁屏记录顶掉 ----

    /// in-memory 库，只建 ot_records 表——避免单测碰真实 `%APPDATA%/niuma-timer/niuma.db`
    fn mem_db() -> Connection {
        let db = Connection::open_in_memory().expect("打开 in-memory 库");
        db.execute_batch(crate::db::CREATE_OT_RECORDS)
            .expect("建 ot_records 表");
        db
    }

    /// 构造一条记录（默认自动来源）
    fn rec_at(date: &str, lock_time: &str, valid_hours: f64) -> OvertimeRecord {
        OvertimeRecord {
            date: date.into(),
            lock_time: lock_time.into(),
            ot_start: "18:00".into(),
            raw_hours: valid_hours,
            valid_hours,
            fee: valid_hours * 20.0,
            meal: 20.0,
            total: valid_hours * 20.0 + 20.0,
            source: SOURCE_AUTO,
        }
    }

    fn query_one(db: &Connection, date: &str) -> OvertimeRecord {
        db.query_row(
            "SELECT date, lock_time, ot_start, raw_hours, valid_hours, fee, meal, total, source \
             FROM ot_records WHERE date = ?1",
            params![date],
            |r| {
                Ok(OvertimeRecord {
                    date: r.get(0)?,
                    lock_time: r.get(1)?,
                    ot_start: r.get(2)?,
                    raw_hours: r.get(3)?,
                    valid_hours: r.get(4)?,
                    fee: r.get(5)?,
                    meal: r.get(6)?,
                    total: r.get(7)?,
                    source: r.get(8)?,
                })
            },
        )
        .expect("应查到记录")
    }

    #[test]
    fn auto_upsert_never_overwrites_manual_record() {
        let db = mem_db();
        // 用户手动把 8/19 改成 21:30 下班（3.5h）
        let mut manual = rec_at("2026-08-19", "21:30", 3.5);
        manual.source = SOURCE_MANUAL;
        upsert_into(&db, &manual, true).unwrap();

        // 当晚再次锁屏，自动算出 20:00 / 2h —— 绝不能顶掉手改值
        let auto = rec_at("2026-08-19", "20:00", 2.0);
        upsert_into(&db, &auto, false).unwrap();

        let got = query_one(&db, "2026-08-19");
        assert_eq!(got.lock_time, "21:30", "手改记录被自动记录覆盖了");
        assert!(approx(got.valid_hours, 3.5));
        assert_eq!(got.source, SOURCE_MANUAL);
    }

    #[test]
    fn auto_upsert_overwrites_previous_auto_record() {
        let db = mem_db();
        // 同日两次锁屏：后一次为准（自动覆盖自动是允许的）
        upsert_into(&db, &rec_at("2026-08-19", "20:00", 2.0), false).unwrap();
        upsert_into(&db, &rec_at("2026-08-19", "21:00", 3.0), false).unwrap();
        let got = query_one(&db, "2026-08-19");
        assert_eq!(got.lock_time, "21:00");
        assert_eq!(got.source, SOURCE_AUTO);
    }

    #[test]
    fn manual_upsert_overwrites_anything() {
        // 手动覆盖手动：手改可以反复改
        let db = mem_db();
        for (t, h) in [("21:30", 3.5), ("22:00", 4.0)] {
            let mut m = rec_at("2026-08-19", t, h);
            m.source = SOURCE_MANUAL;
            upsert_into(&db, &m, true).unwrap();
        }
        assert_eq!(query_one(&db, "2026-08-19").lock_time, "22:00");

        // 手动覆盖自动：手改发生在自动记录之后也要生效
        let db2 = mem_db();
        upsert_into(&db2, &rec_at("2026-08-19", "20:00", 2.0), false).unwrap();
        let mut m = rec_at("2026-08-19", "22:30", 4.5);
        m.source = SOURCE_MANUAL;
        upsert_into(&db2, &m, true).unwrap();
        let got = query_one(&db2, "2026-08-19");
        assert_eq!(got.lock_time, "22:30");
        assert_eq!(got.source, SOURCE_MANUAL);
    }
}
