use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// 应用配置（持久化到 AppData/niuma-timer/config.json）
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// 月薪（元）
    pub monthly_salary: f64,
    /// 上午上班  "HH:MM"
    pub am_start: String,
    /// 上午下班  "HH:MM"
    pub am_end: String,
    /// 下午上班  "HH:MM"
    pub pm_start: String,
    /// 下午下班  "HH:MM"
    pub pm_end: String,
    /// 手动覆盖当月实际上班天数；None = 用自动计算
    pub workdays_override: Option<u32>,
    /// 发薪日（每月几号），用于距发薪日倒计时
    pub payday: u32,
    /// 时长显示格式：hms=几小时几分几秒（默认） hm=几小时几分 h=小数小时
    #[serde(default = "default_duration_format")]
    pub duration_format: String,
    /// 缓存年份标记（保留字段，便于迁移）
    pub last_holiday_year: i32,
}

fn default_duration_format() -> String {
    "hms".into()
}

impl Default for Config {
    fn default() -> Self {
        Config {
            monthly_salary: 15000.0,
            am_start: "09:00".into(),
            am_end: "12:00".into(),
            pm_start: "13:00".into(),
            pm_end: "18:00".into(),
            workdays_override: None,
            payday: 10,
            duration_format: "hms".into(),
            last_holiday_year: 0,
        }
    }
}

/// 配置目录：%APPDATA%/niuma-timer
pub fn config_dir() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("niuma-timer")
}

fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

/// 读取配置；不存在则用默认值并写盘
pub fn load() -> Config {
    let path = config_path();
    if let Ok(s) = fs::read_to_string(&path) {
        if let Ok(cfg) = serde_json::from_str::<Config>(&s) {
            return cfg;
        }
    }
    let cfg = Config::default();
    save(&cfg);
    cfg
}

/// 写入配置
pub fn save(cfg: &Config) {
    let dir = config_dir();
    let _ = fs::create_dir_all(&dir);
    if let Ok(s) = serde_json::to_string_pretty(cfg) {
        let _ = fs::write(config_path(), s);
    }
}
