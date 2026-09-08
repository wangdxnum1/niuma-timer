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

/// 内置法定节假日兜底：国务院办公厅公布的放假调休安排里「特殊」的那几天。
///
/// 为什么需要：当月工作日数直接决定 ¥/分 速率与「今日已赚」，而 `fetch_year`
/// 依赖网络。断网 + 无缓存时旧逻辑退回 `weekday_count`（只按周一至周五估算），
/// 2026 年 10 月会把实际的 18 天算成 22 天，速率偏低约 18%——用户看到的钱是错的。
///
/// 只列特殊日（放假区间 / 调休补班日），其余按周一至周五=工作日推算，
/// 与网络数据共用 `expand_year`，两者语义保证一致。
///
/// **维护**：国务院通常在上一年 11 月公布次年安排，届时在此补一条记录即可。
/// 未收录的年份自动退回 `weekday_count`，并在 debug.log 留一条提醒。
struct BuiltinYear {
    year: i32,
    /// 放假（法定节假日 + 调休连休），闭区间
    offs: &'static [(&'static str, &'static str)],
    /// 调休补班（本该休息却要上班）
    works: &'static [&'static str],
}

/// 数据来源：国务院办公厅《关于部分节假日安排的通知》原文。
const BUILTIN: &[BuiltinYear] = &[
    BuiltinYear {
        year: 2025,
        offs: &[
            ("2025-01-01", "2025-01-01"), // 元旦 1 天，不调休
            ("2025-01-28", "2025-02-04"), // 春节 8 天（除夕起）
            ("2025-04-04", "2025-04-06"), // 清明
            ("2025-05-01", "2025-05-05"), // 劳动节 5 天
            ("2025-05-31", "2025-06-02"), // 端午
            ("2025-10-01", "2025-10-08"), // 国庆 + 中秋合并 8 天
        ],
        works: &[
            "2025-01-26", // 春节前补班（周日）
            "2025-02-08", // 春节后补班（周六）
            "2025-04-27", // 劳动节前补班（周日）
            "2025-09-28", // 国庆前补班（周日）
            "2025-10-11", // 国庆后补班（周六）
        ],
    },
    BuiltinYear {
        year: 2026,
        offs: &[
            ("2026-01-01", "2026-01-03"), // 元旦
            ("2026-02-15", "2026-02-23"), // 春节 9 天（腊月二十八起）
            ("2026-04-04", "2026-04-06"), // 清明
            ("2026-05-01", "2026-05-05"), // 劳动节 5 天
            ("2026-06-19", "2026-06-21"), // 端午
            ("2026-09-25", "2026-09-27"), // 中秋
            ("2026-10-01", "2026-10-07"), // 国庆 7 天
        ],
        works: &[
            "2026-01-04", // 元旦后补班（周日）
            "2026-02-14", // 春节前补班（周六）
            "2026-02-28", // 春节后补班（周六）
            "2026-05-09", // 劳动节后补班（周六）
            "2026-09-20", // 国庆前补班（周日）
            "2026-10-10", // 国庆后补班（周六）
        ],
    },
];

/// 内置兜底：生成某年的放假缓存。未收录的年份返回 None。
///
/// 不落盘：内置表每次启动现算（几十微秒），且不该被当成「权威缓存」写进
/// `holiday_<year>.json` 污染磁盘——联网成功后网络数据自会覆盖内存态。
pub fn builtin_cache(year: i32) -> Option<HolidayCache> {
    let b = BUILTIN.iter().find(|b| b.year == year)?;
    let mut overrides: HashMap<NaiveDate, bool> = HashMap::new();
    for (start, end) in b.offs {
        let mut d = NaiveDate::parse_from_str(start, "%Y-%m-%d").ok()?;
        let last = NaiveDate::parse_from_str(end, "%Y-%m-%d").ok()?;
        loop {
            if d > last {
                break;
            }
            overrides.insert(d, true);
            d = d.succ_opt()?;
        }
    }
    for s in b.works {
        let d = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
        overrides.insert(d, false);
    }
    let days = expand_year(&overrides, year);
    if days.is_empty() {
        return None;
    }
    Some(HolidayCache {
        year,
        fetched_at: 0, // 0 = 非网络来源
        days,
    })
}

/// 兜底：当月自然工作日（周一到周五），无网络/无缓存/无内置表时使用
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

    let map = expand_year(&overrides, data_year);
    if map.is_empty() {
        return Err("未解析到任何日期".into());
    }
    Ok(map)
}

/// 由「特殊日覆盖表」（true=放假 / false=补班）推算出全年每天类型。
/// 网络数据与内置兜底表共用此逻辑，避免两套推算各错各的。
fn expand_year(overrides: &HashMap<NaiveDate, bool>, year: i32) -> HashMap<NaiveDate, u8> {
    let mut map = HashMap::new();
    for month in 1..=12u32 {
        let dim = days_in_month(year, month);
        for day in 1..=dim {
            if let Some(dt) = NaiveDate::from_ymd_opt(year, month, day) {
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
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// 2026 年 10 月：国庆 1-7 号放假、10 号（周六）补班，实际 18 个工作日。
    /// 这正是内置表要修的问题——旧兜底 weekday_count 会算成 22 天，速率偏低约 18%。
    #[test]
    fn builtin_october_2026_is_18_workdays() {
        let c = builtin_cache(2026).expect("2026 应有内置表");
        assert_eq!(c.month_workdays(2026, 10), Some(18));
        assert_eq!(weekday_count(2026, 10), 22);
        // 2025 年 10 月（国庆中秋连休 8 天 + 11 号补班）同样是 18 天
        let c25 = builtin_cache(2025).unwrap();
        assert_eq!(c25.month_workdays(2025, 10), Some(18));
    }

    /// 放假区间内的日子标记为法定节假日(3)，调休补班日标记为补班(2)
    #[test]
    fn builtin_spring_festival_and_makeup_day() {
        let c = builtin_cache(2026).unwrap();
        assert_eq!(c.days[&d("2026-02-17")], 3); // 正月初一
        assert_eq!(c.is_workday(d("2026-02-17")), Some(false));
        assert_eq!(c.days[&d("2026-02-14")], 2); // 调休补班（周六）
        assert_eq!(c.is_workday(d("2026-02-14")), Some(true));
        assert_eq!(c.days[&d("2026-02-28")], 2); // 调休补班（周六）
        assert_eq!(c.is_workday(d("2026-02-28")), Some(true));
    }

    /// 未收录的年份返回 None，调用方据此退回 weekday_count
    #[test]
    fn builtin_missing_year_is_none() {
        assert!(builtin_cache(2030).is_none());
    }

    /// 内置表必须覆盖全年每一天，缺一天就会让 month_workdays 少算一天
    #[test]
    fn builtin_covers_whole_year() {
        for y in [2025, 2026] {
            let c = builtin_cache(y).unwrap();
            let dim = NaiveDate::from_ymd_opt(y, 12, 31).unwrap().ordinal() as usize;
            assert_eq!(c.days.len(), dim, "{y} 年天数不完整");
        }
    }
}
