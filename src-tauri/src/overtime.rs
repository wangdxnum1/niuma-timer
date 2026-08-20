//! 加班记录：数据结构、费用计算、overtime.json 持久化。

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime, TimeZone, Timelike};
use serde::{Deserialize, Serialize};

use crate::config::Config;

/// 单日加班记录
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OvertimeRecord {
    /// 日期 "2026-08-19"
    pub date: String,
    /// 最后锁屏时间 "20:30"
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

/// 根据锁屏时间和配置计算单日加班记录（自动锁屏路径）
/// 返回 None 表示无有效加班（锁屏早于起算时间、不足 1 小时等）
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
    let ot_start_min = to_min(ot_start_str)?;

    let lock_min = lock_time.hour() as f64 * 60.0
        + lock_time.minute() as f64
        + lock_time.second() as f64 / 60.0;

    // 锁屏必须晚于起算时间
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
    })
}

/// 手动录入请求：前端提交日期 + 锁屏时间（+ 可选起算时间覆盖）
#[derive(Clone, Debug, Deserialize)]
pub struct ManualOvertimeInput {
    /// 日期 "2026-08-19"
    pub date: String,
    /// 锁屏时间（最后离开时间）"20:30"
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
    let lock_dt = parse_lock_datetime(&input.date, &input.lock_time)
        .ok_or_else(|| "锁屏时间格式错误，应为 HH:MM".to_string())?;
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
            "锁屏时间 {} 需晚于加班起算时间 {}",
            input.lock_time, ot_start_str
        ));
    }
    let raw_hours = (lock_min - ot_start_min) / 60.0;
    if calc_valid_hours(raw_hours) < 1.0 {
        return Err("加班时长不足 1 小时，无法生成有效记录".to_string());
    }
    let rec = compute_record(date, lock_dt, ot_start_str, cfg)
        .ok_or_else(|| "无法生成加班记录".to_string())?;
    upsert_record(rec.clone());
    Ok(rec)
}

/// 手动删除某天加班记录。仅允许当前月份。
pub fn delete_manual(date: &str) -> Result<(), String> {
    if !is_current_month(date) {
        return Err("只能删除当月的数据".to_string());
    }
    let d = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|_| "日期格式错误".to_string())?;
    let mut all = load_all();
    let key = month_key(d);
    let removed = if let Some(m) = all.get_mut(&key) {
        let before = m.records.len();
        m.records.retain(|r| r.date != date);
        before != m.records.len()
    } else {
        false
    };
    if !removed {
        return Err("未找到该日期的加班记录".to_string());
    }
    save_all(&all);
    Ok(())
}

// ---- 持久化 ----

fn overtime_path() -> PathBuf {
    crate::config::config_dir().join("overtime.json")
}

fn load_all() -> BTreeMap<String, MonthlyOvertime> {
    let path = overtime_path();
    if let Ok(s) = fs::read_to_string(&path) {
        if let Ok(map) = serde_json::from_str::<BTreeMap<String, MonthlyOvertime>>(&s) {
            return map;
        }
    }
    BTreeMap::new()
}

fn save_all(map: &BTreeMap<String, MonthlyOvertime>) {
    let dir = crate::config::config_dir();
    let _ = fs::create_dir_all(&dir);
    if let Ok(s) = serde_json::to_string_pretty(map) {
        let _ = fs::write(overtime_path(), s);
    }
}

/// 月份键 "2026-08"
pub fn month_key(date: NaiveDate) -> String {
    format!("{:04}-{:02}", date.year(), date.month())
}

/// 添加或更新某天的加班记录（按日期去重，同日覆盖）
pub fn upsert_record(record: OvertimeRecord) {
    let date = NaiveDate::parse_from_str(&record.date, "%Y-%m-%d").ok();
    let key = match date {
        Some(d) => month_key(d),
        None => return,
    };
    let mut all = load_all();
    let monthly = all.entry(key).or_default();

    if let Some(r) = monthly.records.iter_mut().find(|r| r.date == record.date) {
        *r = record;
    } else {
        monthly.records.push(record);
        monthly.records.sort_by(|a, b| a.date.cmp(&b.date));
    }
    save_all(&all);
}

/// 获取指定月份的加班记录
pub fn get_month(year: i32, month: u32) -> MonthlyOvertime {
    let key = format!("{:04}-{:02}", year, month);
    let all = load_all();
    all.get(&key).cloned().unwrap_or_default()
}
