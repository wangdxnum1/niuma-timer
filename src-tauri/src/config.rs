use chrono::Local;
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
    /// 托盘悬停显示彩色卡片（true=自绘 HTML 卡片，false=系统原生 tooltip），默认开启
    #[serde(default = "default_true")]
    pub tray_hover_card: bool,

    // ---- 加班追踪 ----
    /// 加班追踪开关
    #[serde(default)]
    pub overtime_enabled: bool,
    /// 加班起算时间 "HH:MM"，None=用 pm_end
    #[serde(default)]
    pub overtime_start: Option<String>,
    /// 加班费（元/小时）
    #[serde(default = "default_overtime_rate")]
    pub overtime_rate: f64,
    /// 饭补开关
    #[serde(default = "default_overtime_meal_enabled")]
    pub overtime_meal_enabled: bool,
    /// 饭补金额（元）
    #[serde(default = "default_overtime_meal")]
    pub overtime_meal: f64,
    /// 周末加班开关：周六/周日是否计入加班（默认关闭；开启后周末下班晚也记加班）
    #[serde(default)]
    pub weekend_overtime: bool,

    // ---- 应用使用白名单 ----
    /// 仅统计白名单内应用（关闭或空名单=统计全部前台应用，兼容现状）
    #[serde(default)]
    pub app_whitelist_enabled: bool,
    /// 应用名白名单（展示名，大小写不敏感）；仅当 app_whitelist_enabled=true 且非空时生效
    #[serde(default)]
    pub app_whitelist: Vec<String>,

    // ---- 功能监控开关（关闭后对应线程空转，几乎不占 CPU）----
    /// 鼠标键盘活动监控
    #[serde(default = "default_true")]
    pub monitor_activity: bool,
    /// 应用使用监控
    #[serde(default = "default_true")]
    pub monitor_app_usage: bool,
    /// 媒体播放监控
    #[serde(default = "default_true")]
    pub monitor_audio: bool,

    // ---- 首页副标题文案 ----
    /// 副标题风格：dynamic=随状态变化（默认，推荐）
    /// price / rise / count / classic = 固定文案
    /// none = 不显示这一行   custom = 显示 tagline_custom
    #[serde(default = "default_tagline_style")]
    pub tagline_style: String,
    /// 自定义副标题文案；仅当 tagline_style == "custom" 时生效。
    /// 纯前端消费，后端不参与计算。
    #[serde(default)]
    pub tagline_custom: String,
}

fn default_true() -> bool {
    true
}

fn default_overtime_rate() -> f64 {
    20.0
}
fn default_overtime_meal_enabled() -> bool {
    true
}
fn default_overtime_meal() -> f64 {
    20.0
}

fn default_duration_format() -> String {
    "hms".into()
}

fn default_tagline_style() -> String {
    "dynamic".into()
}

impl Default for Config {
    fn default() -> Self {
        Config {
            monthly_salary: 10000.0,
            am_start: "09:00".into(),
            am_end: "12:00".into(),
            pm_start: "13:00".into(),
            pm_end: "18:00".into(),
            workdays_override: None,
            payday: 10,
            duration_format: "hms".into(),
            tray_hover_card: true,
            overtime_enabled: false,
            overtime_start: None,
            overtime_rate: 20.0,
            overtime_meal_enabled: true,
            overtime_meal: 20.0,
            weekend_overtime: false,
            app_whitelist_enabled: false,
            app_whitelist: Vec::new(),
            monitor_activity: true,
            monitor_app_usage: true,
            monitor_audio: true,
            tagline_style: "dynamic".into(),
            tagline_custom: String::new(),
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

/// 读取配置；不存在则用默认值并写盘。
/// 文件存在但解析失败（损坏 / 写到一半）→ 先备份为 config.json.corrupt-时间戳.bak，
/// 再回退默认配置，**绝不用默认值直接覆盖**，避免静默丢失用户配置。
pub fn load() -> Config {
    let path = config_path();
    if let Ok(s) = fs::read_to_string(&path) {
        if let Ok(cfg) = serde_json::from_str::<Config>(&s) {
            return cfg;
        }
        // 解析失败：原文件损坏，先备份再回退默认
        let ts = Local::now().format("%Y%m%d-%H%M%S");
        let bak = path.with_file_name(format!("config.json.corrupt-{ts}.bak"));
        let _ = fs::rename(&path, &bak);
        eprintln!(
            "[config] config.json 解析失败，已备份为 {}（回退默认配置）",
            bak.display()
        );
    }
    let cfg = Config::default();
    save(&cfg);
    cfg
}

/// 写入配置（原子写：先写临时文件再 rename 覆盖）。
/// 避免写到一半崩溃 / 断电，残留半截文件导致下次启动解析失败、整份配置被默认值替换。
/// Windows 下 rename 不能覆盖已存在目标，故先删旧文件再移动（极小时间窗，单机可接受）；
/// 任一环节失败均退回直接覆盖写，至少保证内存态落盘。
pub fn save(cfg: &Config) {
    let dir = config_dir();
    let _ = fs::create_dir_all(&dir);
    if let Ok(s) = serde_json::to_string_pretty(cfg) {
        let tmp = config_path().with_extension("json.tmp");
        if fs::write(&tmp, &s).is_ok() {
            let _ = fs::remove_file(config_path());
            if fs::rename(&tmp, config_path()).is_ok() {
                return;
            }
            // rename 失败（如跨卷）：退回直接覆盖写
            let _ = fs::write(config_path(), &s);
        }
    }
}
