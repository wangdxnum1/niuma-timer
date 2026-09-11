//! 加班记录：数据结构、费用计算、SQLite 持久化（ot_records 表）。

use crate::holiday::HolidayCache;
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
    /// 这份视图对应的年份（前端标题直接用它，避免前后端各算一次导致不一致）
    pub year: i32,
    /// 这份视图对应的月份（1-12）
    pub month: u32,
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

    /// 转为前端响应视图（records + 预计算汇总）。
    /// year/month 由调用方传入：MonthlyOvertime 只存 records，自己不知道是哪个月。
    pub fn to_view(&self, year: i32, month: u32) -> MonthlyOvertimeView {
        MonthlyOvertimeView {
            year,
            month,
            total_hours: self.total_hours(),
            total_fee: self.total_fee(),
            total_meal: self.total_meal(),
            total_all: self.total_all(),
            days: self.days(),
            records: self.records.clone(),
        }
    }
}

/// 加班日类型，决定「起算时间」与「费率」两件事
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DayKind {
    /// 工作日；含调休补班日（那天的班是正常的，加班按工作日规则算）
    Workday,
    /// 普通周末，含因法定假日调休而来的休息日
    Weekend,
    /// 法定节假日
    Holiday,
}

impl DayKind {
    /// 是否非工作日（受 weekend_overtime 开关约束）
    pub fn is_rest(self) -> bool {
        !matches!(self, DayKind::Workday)
    }
    pub fn label(self) -> &'static str {
        match self {
            DayKind::Workday => "工作日",
            DayKind::Weekend => "休息日",
            DayKind::Holiday => "法定节假日",
        }
    }
}

/// 休息日加班的默认起算时间。
/// 休息日没有「下班时间」概念，沿用工作日的 pm_end（18:00）会让上午来、下午 5 点走的人
/// 因为 `lock_min <= ot_start` 而一分钱算不到，且毫无提示——默认给 09:00 才符合实际。
pub const DEFAULT_REST_OT_START: &str = "09:00";

/// 判定某天属于哪一类。节假日数据缺失（未联网且无内置表）时降级为按自然周几判断。
pub fn day_kind(date: NaiveDate, holiday: Option<&HolidayCache>) -> DayKind {
    if let Some(c) = holiday {
        // 0=工作日 1=周末 2=补班 3=法定节假日
        match c.day_type(date) {
            Some(0) | Some(2) => return DayKind::Workday,
            Some(1) => return DayKind::Weekend,
            Some(3) => return DayKind::Holiday,
            _ => {}
        }
    }
    if matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
        DayKind::Weekend
    } else {
        DayKind::Workday
    }
}

/// 该类型的加班起算时间：休息日/法定节假日有独立配置，工作日沿用原有规则
fn ot_start_of(cfg: &Config, kind: DayKind) -> &str {
    match kind {
        DayKind::Workday => cfg.overtime_start.as_deref().unwrap_or(&cfg.pm_end),
        _ => cfg
            .weekend_ot_start
            .as_deref()
            .unwrap_or(DEFAULT_REST_OT_START),
    }
}

/// 该类型的加班费率（元/小时）。未单独配置时逐级回退，老配置不受影响：
/// 法定节假日 → 休息日 → 工作日
fn rate_of(cfg: &Config, kind: DayKind) -> f64 {
    match kind {
        DayKind::Workday => cfg.overtime_rate,
        DayKind::Weekend => cfg.overtime_rate_weekend.unwrap_or(cfg.overtime_rate),
        DayKind::Holiday => cfg
            .overtime_rate_holiday
            .or(cfg.overtime_rate_weekend)
            .unwrap_or(cfg.overtime_rate),
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
    holiday: Option<&HolidayCache>,
) -> Option<OvertimeRecord> {
    compute_record(date, lock_time, None, cfg, day_kind(date, holiday))
}

/// 核心计算：给定明确的加班起算时间字符串，计算单日记录。
/// 抽出来供自动锁屏路径与手动录入路径共用，保证规则一致。
///
/// `ot_start_override` 只在手动录入时由调用方给出（用户在表单里覆盖起算时间），
/// 否则按 [`ot_start_of`] 依日期类型选取。
fn compute_record(
    date: NaiveDate,
    lock_time: DateTime<Local>,
    ot_start_override: Option<&str>,
    cfg: &Config,
    kind: DayKind,
) -> Option<OvertimeRecord> {
    // 非工作日受配置开关约束：未开启 weekend_overtime 时休息日/法定节假日不计加班。
    // 工作日（含调休补班日）不受此开关影响。
    if kind.is_rest() && !cfg.weekend_overtime {
        return None;
    }

    let ot_start_str = ot_start_override.unwrap_or_else(|| ot_start_of(cfg, kind));
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

    let fee = valid_hours * rate_of(cfg, kind);
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
/// 从 "YYYY-MM-DD" 取 (年, 月)；格式错误返回 None
pub fn month_of(date: &str) -> Option<(i32, u32)> {
    let d = NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    Some((d.year(), d.month()))
}

/// 日期是否晚于今天。加班是既成事实，不允许补录未来日期；
/// 但允许补录/修正任意历史月份——界面已支持切月，再把写入锁死在当月没有意义。
/// 日期解析不了时也返回 true（当非法日期拒绝）。
fn is_future(date: &str) -> bool {
    match NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        Ok(d) => d > Local::now().date_naive(),
        Err(_) => true,
    }
}

/// 手动添加/修改某天加班记录（按日期 upsert）。
/// 仅允许当前月份；返回生成的记录或错误信息。
pub fn save_manual(
    input: ManualOvertimeInput,
    cfg: &Config,
    holiday: Option<&HolidayCache>,
) -> Result<OvertimeRecord, String> {
    let date = NaiveDate::parse_from_str(&input.date, "%Y-%m-%d")
        .map_err(|_| "日期格式错误".to_string())?;
    if is_future(&input.date) {
        return Err("不能添加未来日期的加班记录".to_string());
    }
    let kind = day_kind(date, holiday);
    // 非工作日受开关约束：手动录入休息日/法定节假日且未开启时给出明确提示
    if kind.is_rest() && !cfg.weekend_overtime {
        return Err(format!(
            "{}加班功能未开启，请在设置中开启「休息日 / 节假日加班」",
            kind.label()
        ));
    }
    let lock_dt = parse_lock_datetime(&input.date, &input.lock_time)
        .ok_or_else(|| "下班时间格式错误，应为 HH:MM".to_string())?;
    // 起算时间：手动覆盖 > 按日期类型选取（工作日 overtime_start/pm_end、休息日 weekend_ot_start）
    let ot_start_str = input
        .ot_start
        .as_deref()
        .unwrap_or_else(|| ot_start_of(cfg, kind));
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
    let rec = compute_record(date, lock_dt, Some(ot_start_str), cfg, kind)
        .ok_or_else(|| "无法生成加班记录".to_string())?;
    // 标为手动来源：此后自动锁屏记录不再覆盖这一天（见 upsert_auto）
    upsert_manual(rec.clone());
    Ok(rec)
}

/// 手动删除某天加班记录。仅允许当前月份。
pub fn delete_manual(date: &str) -> Result<(), String> {
    // 历史月份的记录同样允许删除（界面可切月查看），只挡未来日期
    if is_future(date) {
        return Err("不能删除未来日期的记录".to_string());
    }
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|_| "日期格式错误".to_string())?;
    let n = crate::db::with_db(|g| {
        g.execute("DELETE FROM ot_records WHERE date = ?1", params![date])
    })
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
    let _ = crate::db::with_db(|g| upsert_into(g, &rec, false));
}

/// **手动**路径 upsert（加班明细页增删改）。手改是用户明确意图，优先级最高，无条件覆盖。
pub fn upsert_manual(record: OvertimeRecord) {
    let mut rec = record;
    rec.source = SOURCE_MANUAL;
    let _ = crate::db::with_db(|g| upsert_into(g, &rec, true));
}

/// 获取指定月份的加班记录（SQL 按日期前缀过滤，历史月与当月同表，无需归档）。
pub fn get_month(year: i32, month: u32) -> MonthlyOvertime {
    let prefix = format!("{:04}-{:02}-", year, month);
    // 查询失败（表未建 / SQL 出错）时降级为空月报，不再 panic；错误已由 with_db 记入 debug.log
    crate::db::with_db(|g| {
        let mut stmt = g.prepare(
            "SELECT date, lock_time, ot_start, raw_hours, valid_hours, fee, meal, total, source \
             FROM ot_records WHERE date LIKE ?1 ORDER BY date",
        )?;
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
        })?;
        Ok(MonthlyOvertime {
            records: rows.filter_map(|r| r.ok()).collect(),
        })
    })
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, NaiveDate, NaiveTime, TimeZone};
    use std::collections::HashMap;

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
        let r = compute_record(
            date,
            lock_dt(2026, 8, 19, 20, 30),
            Some("18:00"),
            &cfg,
            DayKind::Workday,
        )
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
        assert!(
            compute_record(date, lock_dt(2026, 8, 19, 18, 30), Some("18:00"), &cfg, DayKind::Workday)
                .is_none()
        );
    }

    #[test]
    fn compute_record_weekend_disabled_none() {
        let cfg = Config::default(); // weekend_overtime=false
        let date = NaiveDate::from_ymd_opt(2026, 8, 22).unwrap(); // 周六
        assert!(
            compute_record(date, lock_dt(2026, 8, 22, 22, 0), Some("18:00"), &cfg, DayKind::Weekend)
                .is_none()
        );
    }

    #[test]
    fn compute_record_weekend_enabled_some() {
        let mut cfg = Config::default();
        cfg.weekend_overtime = true;
        let date = NaiveDate::from_ymd_opt(2026, 8, 22).unwrap(); // 周六
        let r = compute_record(
            date,
            lock_dt(2026, 8, 22, 22, 0),
            Some("18:00"),
            &cfg,
            DayKind::Weekend,
        )
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
        let r = compute_record(
            date,
            lock_dt(2026, 8, 19, 20, 30),
            Some("19:00"),
            &cfg,
            DayKind::Workday,
        )
        .expect("应生成");
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
        let r = calc_record(date, lock_dt(2026, 8, 19, 20, 0), &cfg, None).expect("应生成");
        assert_eq!(r.ot_start, "18:00");
        assert!(approx(r.valid_hours, 2.0));
    }

    // ---- 休息日 / 法定节假日：日期类型判定与费率 ----

    /// 构造只有若干天类型标记的节假日缓存
    fn holiday_with(entries: &[(i32, u32, u32, u8)]) -> HolidayCache {
        let mut days = HashMap::new();
        for (y, m, d, t) in entries {
            days.insert(NaiveDate::from_ymd_opt(*y, *m, *d).unwrap(), *t);
        }
        HolidayCache {
            year: 2026,
            fetched_at: 1,
            days,
        }
    }

    #[test]
    fn day_kind_reads_holiday_types() {
        let c = holiday_with(&[
            (2026, 9, 25, 0), // 周五 工作日
            (2026, 9, 26, 1), // 周六 普通周末
            (2026, 9, 27, 2), // 周日 调休补班 -> 按工作日算
            (2026, 10, 1, 3), // 周四 国庆 -> 法定节假日
        ]);
        let d = |y, m, dd| NaiveDate::from_ymd_opt(y, m, dd).unwrap();
        assert_eq!(day_kind(d(2026, 9, 25), Some(&c)), DayKind::Workday);
        assert_eq!(day_kind(d(2026, 9, 26), Some(&c)), DayKind::Weekend);
        assert_eq!(day_kind(d(2026, 9, 27), Some(&c)), DayKind::Workday);
        assert_eq!(day_kind(d(2026, 10, 1), Some(&c)), DayKind::Holiday);
    }

    /// 节假日数据缺失（未联网且无内置表）时降级为按自然周几判断，不误伤
    #[test]
    fn day_kind_falls_back_without_holiday_data() {
        let d = |y, m, dd| NaiveDate::from_ymd_opt(y, m, dd).unwrap();
        assert_eq!(day_kind(d(2026, 8, 22), None), DayKind::Weekend); // 周六
        assert_eq!(day_kind(d(2026, 8, 19), None), DayKind::Workday); // 周三
    }

    /// 修复前的核心 bug：休息日沿用工作日的 18:00 起算，上午来、下午走的人
    /// 因为 `lock_min <= ot_start` 直接 return None，一分钱算不到且无任何提示。
    #[test]
    fn rest_day_starts_at_09_by_default() {
        let mut cfg = Config::default();
        cfg.weekend_overtime = true;
        let date = NaiveDate::from_ymd_opt(2026, 8, 22).unwrap(); // 周六
        let r = calc_record(date, lock_dt(2026, 8, 22, 14, 0), &cfg, None).expect("休息日应生成");
        assert_eq!(r.ot_start, "09:00");
        assert!(approx(r.valid_hours, 5.0));
        assert!(approx(r.fee, 100.0)); // 5h * 20
        assert!(approx(r.total, 120.0)); // + 20 饭补
    }

    /// 开关关闭时休息日不算加班
    #[test]
    fn rest_day_switch_off_means_none() {
        let cfg = Config::default(); // weekend_overtime = false
        let date = NaiveDate::from_ymd_opt(2026, 8, 22).unwrap();
        assert!(calc_record(date, lock_dt(2026, 8, 22, 22, 0), &cfg, None).is_none());
    }

    /// 费率回退链：法定节假日 -> 休息日 -> 工作日。老配置没有新字段时不改变行为
    #[test]
    fn rate_falls_back_chain() {
        let mut cfg = Config::default();
        assert!(approx(rate_of(&cfg, DayKind::Workday), 20.0));
        assert!(approx(rate_of(&cfg, DayKind::Weekend), 20.0));
        assert!(approx(rate_of(&cfg, DayKind::Holiday), 20.0));

        cfg.overtime_rate_weekend = Some(40.0);
        assert!(approx(rate_of(&cfg, DayKind::Weekend), 40.0));
        assert!(approx(rate_of(&cfg, DayKind::Holiday), 40.0)); // 节假日未配 -> 用休息日

        cfg.overtime_rate_holiday = Some(60.0);
        assert!(approx(rate_of(&cfg, DayKind::Holiday), 60.0));
        assert!(approx(rate_of(&cfg, DayKind::Workday), 20.0)); // 工作日不受影响
    }

    /// 法定节假日：日期类型来自 holiday 数据（周三国庆），费率走节假日档
    #[test]
    fn holiday_uses_holiday_rate() {
        let mut cfg = Config::default();
        cfg.weekend_overtime = true;
        cfg.overtime_rate_holiday = Some(60.0);
        let c = holiday_with(&[(2026, 10, 1, 3)]);
        let date = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let r = calc_record(date, lock_dt(2026, 10, 1, 17, 0), &cfg, Some(&c)).expect("节假日应生成");
        assert_eq!(r.ot_start, "09:00");
        assert!(approx(r.valid_hours, 8.0));
        assert!(approx(r.fee, 480.0)); // 8h * 60
    }

    /// 调休补班日（周日但要上班）按工作日规则：不受 weekend_overtime 开关影响
    #[test]
    fn makeup_workday_ignores_rest_switch() {
        let cfg = Config::default(); // weekend_overtime = false
        let c = holiday_with(&[(2026, 9, 27, 2)]);
        let date = NaiveDate::from_ymd_opt(2026, 9, 27).unwrap(); // 周日补班
        let r = calc_record(date, lock_dt(2026, 9, 27, 21, 0), &cfg, Some(&c))
            .expect("补班日应按工作日算");
        assert_eq!(r.ot_start, "18:00"); // 工作日起算，而非休息日的 09:00
        assert!(approx(r.valid_hours, 3.0));
    }

    /// 手动录入休息日而开关未开时，给出指向开关的明确报错（而不是静默失败）
    #[test]
    fn save_manual_rest_day_requires_switch() {
        let cfg = Config::default(); // weekend_overtime = false
        let input = ManualOvertimeInput {
            date: "2026-08-22".into(), // 周六
            lock_time: "20:00".into(),
            ot_start: None,
        };
        let err = save_manual(input, &cfg, None).unwrap_err();
        assert!(err.contains("休息日"), "报错应说明是休息日，实际: {err}");
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
