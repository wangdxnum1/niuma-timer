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

/// 拉取某年节假日数据。
/// 数据源：NateScarlet/holiday-cn（国务院放假安排），经 jsDelivr CDN 分发（国内可达）。
/// 返回全年每天的类型映射：0=工作日 1=周末 2=补班 3=法定节假日。
pub async fn fetch_year(year: i32) -> Result<HashMap<NaiveDate, u8>, String> {
    // 主源 + 备用 CDN（同份数据，不同边缘节点）
    let urls = [
        format!("https://cdn.jsdelivr.net/gh/NateScarlet/holiday-cn@master/{}.json", year),
        format!("https://fastly.jsdelivr.net/gh/NateScarlet/holiday-cn@master/{}.json", year),
        format!("https://gcore.jsdelivr.net/gh/NateScarlet/holiday-cn@master/{}.json", year),
    ];

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) niuma-timer")
        .build()
        .map_err(|e| format!("客户端初始化失败: {}", e))?;

    let mut last_err = String::new();
    for url in &urls {
        match client.get(url).send().await {
            Ok(resp) => {
                if !resp.status().is_success() {
                    last_err = format!("HTTP {}", resp.status());
                    continue;
                }
                let json: serde_json::Value = match resp.json().await {
                    Ok(j) => j,
                    Err(e) => {
                        last_err = format!("数据解析失败: {}", e);
                        continue;
                    }
                };
                match parse_holiday_cn(&json, year) {
                    Ok(map) => return Ok(map),
                    Err(e) => last_err = e,
                }
            }
            Err(e) => {
                last_err = format!("网络请求失败: {}", e);
                continue;
            }
        }
    }
    Err(format!("节假日数据获取失败（已尝试多个数据源）: {}", last_err))
}

/// 解析 NateScarlet/holiday-cn 格式：
/// days 为数组，每项 { name, date:"YYYY-MM-DD", isOffDay:bool }
/// isOffDay=true → 法定节假日(休息)；isOffDay=false → 补班(上班)
/// 仅列出"特殊日"，需结合星期推算出全年每天类型。
fn parse_holiday_cn(json: &serde_json::Value, year: i32) -> Result<HashMap<NaiveDate, u8>, String> {
    let data_year = json
        .get("year")
        .and_then(|v| v.as_i64())
        .unwrap_or(year as i64) as i32;

    // 收集特殊日覆盖：date -> isOffDay
    let mut overrides: HashMap<NaiveDate, bool> = HashMap::new();
    if let Some(arr) = json.get("days").and_then(|v| v.as_array()) {
        for item in arr {
            let date_str = item.get("date").and_then(|v| v.as_str());
            let is_off = item.get("isOffDay").and_then(|v| v.as_bool()).unwrap_or(false);
            if let Some(s) = date_str {
                if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                    overrides.insert(d, is_off);
                }
            }
        }
    }

    let mut map = HashMap::new();
    for month in 1..=12u32 {
        let dim = days_in_month(data_year, month);
        for day in 1..=dim {
            if let Some(dt) = NaiveDate::from_ymd_opt(data_year, month, day) {
                let wd = dt.weekday().num_days_from_monday(); // 0=Mon
                // 基础：周一到周五=工作日(0)，周六日=周末(1)
                let mut t: u8 = if wd < 5 { 0 } else { 1 };
                // 覆盖：法定节假日(3) 或 补班(2)
                if let Some(is_off) = overrides.get(&dt) {
                    t = if *is_off { 3 } else { 2 };
                }
                map.insert(dt, t);
            }
        }
    }

    if map.is_empty() {
        return Err("未解析到任何日期".into());
    }
    Ok(map)
}
