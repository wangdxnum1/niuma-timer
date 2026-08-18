use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::config;

/// 节假日缓存：当年每天的类型
/// type: 0=工作日 1=周末 2=补班 3=法定节假日
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct HolidayCache {
    pub year: i32,
    pub fetched_at: i64,
    pub days: HashMap<NaiveDate, u8>,
}

impl HolidayCache {
    /// 该日期是否为工作日（班/补班）。未知返回 None
    pub fn is_workday(&self, date: NaiveDate) -> Option<bool> {
        self.days
            .get(&date)
            .map(|t| *t == 0 || *t == 2)
    }

    /// 当月实际上班天数（班+补班）。年份不符或无数据返回 None
    pub fn month_workdays(&self, year: i32, month: u32) -> Option<u32> {
        if self.year != year || self.days.is_empty() {
            return None;
        }
        let dim = days_in_month(year, month);
        let mut count = 0u32;
        for d in 1..=dim {
            if let Some(dt) = NaiveDate::from_ymd_opt(year, month, d) {
                if let Some(t) = self.days.get(&dt) {
                    if *t == 0 || *t == 2 {
                        count += 1;
                    }
                }
            }
        }
        if count == 0 {
            None
        } else {
            Some(count)
        }
    }
}

fn days_in_month(y: i32, m: u32) -> u32 {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    NaiveDate::from_ymd_opt(ny, nm, 1).unwrap().pred_opt().expect("valid date").day()
}

fn cache_path(year: i32) -> PathBuf {
    config::config_dir().join(format!("holiday_{}.json", year))
}

/// 读取本地缓存
pub fn load_cache(year: i32) -> Option<HolidayCache> {
    let s = fs::read_to_string(cache_path(year)).ok()?;
    let c = serde_json::from_str::<HolidayCache>(&s).ok()?;
    Some(c)
}

/// 写入本地缓存
pub fn save_cache(c: &HolidayCache) {
    let _ = fs::create_dir_all(config::config_dir());
    if let Ok(s) = serde_json::to_string(c) {
        let _ = fs::write(cache_path(c.year), s);
    }
}

/// 兜底：当月自然工作日（周一到周五），无网络/无缓存时使用
pub fn weekday_count(year: i32, month: u32) -> u32 {
    let dim = days_in_month(year, month);
    let mut count = 0u32;
    for d in 1..=dim {
        if let Some(dt) = NaiveDate::from_ymd_opt(year, month, d) {
            let wd = dt.weekday().num_days_from_monday(); // 0=Mon
            if wd < 5 {
                count += 1;
            }
        }
    }
    count
}

/// 拉取某年节假日数据（timor.tech）
pub async fn fetch_year(year: i32) -> Result<HashMap<NaiveDate, u8>, String> {
    let url = format!("https://timor.tech/api/holiday/year/{}", year);
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析失败: {}", e))?;

    let mut map = HashMap::new();
    if let Some(obj) = json.get("holiday").and_then(|v| v.as_object()) {
        for (k, v) in obj {
            let parts: Vec<&str> = k.split('-').collect();
            if parts.len() == 3 {
                if let (Ok(y), Ok(mo), Ok(d)) = (
                    parts[0].parse::<i32>(),
                    parts[1].parse::<u32>(),
                    parts[2].parse::<u32>(),
                ) {
                    if let Some(dt) = NaiveDate::from_ymd_opt(y, mo, d) {
                        let t = v
                            .get("type")
                            .and_then(|x| x.as_i64())
                            .unwrap_or(1) as u8;
                        map.insert(dt, t);
                    }
                }
            }
        }
    }
    if map.is_empty() {
        return Err("节假日数据为空".into());
    }
    Ok(map)
}
