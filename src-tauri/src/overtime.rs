//! 加班记录：数据结构、费用计算、overtime.json 持久化。

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, Datelike, Local, NaiveDate, Timelike};
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

/// 根据锁屏时间和配置计算单日加班记录
/// 返回 None 表示无有效加班（锁屏早于起算时间、不足 1 小时等）
pub fn calc_record(
    date: NaiveDate,
    lock_time: DateTime<Local>,
    cfg: &Config,
) -> Option<OvertimeRecord> {
    // 加班起算时间：overtime_start 有值则用，否则用 pm_end
    let ot_start_str = cfg.overtime_start.as_deref().unwrap_or(&cfg.pm_end);
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
