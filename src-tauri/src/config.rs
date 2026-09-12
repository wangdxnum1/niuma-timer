use chrono::{Datelike, Local};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

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
    /// 覆盖值所属年月 `"YYYY-MM"`。与 `workdays_override` 成对：换月后自动失效，
    /// 避免 10 月填的 18 一直当 11 月的分母。旧配置缺此字段时，首次加载会戳成当前月。
    #[serde(default)]
    pub workdays_override_for: Option<String>,
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
    /// 休息日 / 节假日加班开关：非工作日是否计入加班（默认关闭）
    ///
    /// 关闭时，周末与法定节假日一律不计加班（调休补班日仍按工作日正常计算）。
    #[serde(default)]
    pub weekend_overtime: bool,
    /// 休息日 / 法定节假日加班的起算时间 "HH:MM"（None = 用 [`DEFAULT_REST_OT_START`]）。
    /// 休息日没有「下班时间」概念，沿用工作日的 pm_end 会让上午来、下午走的人算不到加班，
    /// 故单独给一个默认 09:00 的起算点。
    #[serde(default)]
    pub weekend_ot_start: Option<String>,
    /// 休息日加班费率（元/小时）。None = 沿用 [`Config::overtime_rate`]
    #[serde(default)]
    pub overtime_rate_weekend: Option<f64>,
    /// 法定节假日加班费率（元/小时）。None = 沿用休息日费率，再没有则沿用工作日费率
    #[serde(default)]
    pub overtime_rate_holiday: Option<f64>,

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

    // ---- 数据保留 ----
    /// 历史数据保留天数（加班 / 活动 / 应用 / 媒体，共 7 张表）。
    /// **0 = 永久保留**（默认）：不删任何用户数据。
    ///
    /// 之所以默认不删：数据体量本身很小（实测约 11KB/天，一年 4MB），
    /// 保留策略的真实价值是「用户想清理」，而不是「不清理会爆」。
    /// 与此解耦的是图标缓存——它可从 exe 重新提取，由 maintain 模块按固定
    /// 180 天清理，不受本配置影响。
    #[serde(default)]
    pub retention_days: u32,
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
            workdays_override_for: None,
            payday: 10,
            duration_format: "hms".into(),
            tray_hover_card: true,
            overtime_enabled: false,
            overtime_start: None,
            overtime_rate: 20.0,
            overtime_meal_enabled: true,
            overtime_meal: 20.0,
            weekend_overtime: false,
            weekend_ot_start: None,
            overtime_rate_weekend: None,
            overtime_rate_holiday: None,
            app_whitelist_enabled: false,
            app_whitelist: Vec::new(),
            monitor_activity: true,
            monitor_app_usage: true,
            monitor_audio: true,
            tagline_style: "dynamic".into(),
            tagline_custom: String::new(),
            retention_days: 0,
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

/// 配置装载的四分类。read_config_file 只读+分类、**零写副作用**，
/// 因此能离线单测（见 tests）。
#[derive(Debug)]
enum LoadResult {
    /// 读到有效配置
    Loaded(Config),
    /// 文件在但内容不是合法配置（损坏 / 写到一半）
    Corrupt,
    /// 文件在但读不了（被杀毒/备份软件短暂占用、权限抖动）
    Unreadable(std::io::Error),
    /// 文件不存在（首跑）
    Missing,
}

/// load 的可测内核：读文件 + 解析 + 分类。区分「不存在 / 读不了 / 读到但坏」
/// 三种文件态是本函数存在的全部意义——三者在上层的处置完全不同，混在一个
/// `if let Ok` 里就会把「读不了」错当成「没配置」而覆盖用户数据。
fn read_config_file(path: &Path) -> LoadResult {
    match fs::read_to_string(path) {
        Ok(s) => match serde_json::from_str::<Config>(&s) {
            Ok(cfg) => LoadResult::Loaded(cfg),
            Err(_) => LoadResult::Corrupt,
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => LoadResult::Missing,
        Err(e) => LoadResult::Unreadable(e),
    }
}

/// 读取配置；不存在则用默认值并写盘。
/// - 文件解析失败（损坏 / 写到一半）→ 先备份为 config.json.corrupt-时间戳.bak，
///   再回退默认配置，**绝不用默认值直接覆盖**，避免静默丢失用户配置；
/// - 文件读不了（被占用 / 权限）→ 内存用默认值撑着，**磁盘原样保留**，
///   绝不落盘默认值覆盖真配置——下次启动大概率就能读到了。
pub fn load() -> Config {
    let path = config_path();
    match read_config_file(&path) {
        LoadResult::Loaded(mut cfg) => {
            if stamp_override_month(&mut cfg) {
                save(&cfg);
            }
            cfg
        }
        LoadResult::Corrupt => {
            let ts = Local::now().format("%Y%m%d-%H%M%S");
            let bak = path.with_file_name(format!("config.json.corrupt-{ts}.bak"));
            let _ = fs::rename(&path, &bak);
            eprintln!(
                "[config] config.json 解析失败，已备份为 {}（回退默认配置）",
                bak.display()
            );
            let cfg = Config::default();
            save(&cfg);
            cfg
        }
        LoadResult::Unreadable(e) => {
            eprintln!("[config] config.json 读取失败（{e}），本次先用默认配置运行，不覆盖磁盘原文件");
            crate::db::debug_log(&format!("[config] config.json 读取失败: {e}"));
            Config::default()
        }
        LoadResult::Missing => {
            let cfg = Config::default();
            save(&cfg);
            cfg
        }
    }
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

/// 把前端传来的部分字段合并进现有配置。失败时返回 Err，调用方不得假装保存成功。
pub fn merge_from_value(existing: &Config, incoming: &serde_json::Value) -> Result<Config, String> {
    let mut base = serde_json::to_value(existing).map_err(|e| format!("配置序列化失败: {e}"))?;
    if let Some(obj) = base.as_object_mut() {
        if let Some(inc) = incoming.as_object() {
            for (k, v) in inc {
                obj.insert(k.clone(), v.clone());
            }
        }
    }
    serde_json::from_value(base).map_err(|e| format!("配置合并失败: {e}"))
}

/// 旧配置只有 `workdays_override`、没有所属月：戳成当前月并返回 true（调用方应落盘）。
/// 这样本月覆盖仍生效，下月启动后 `effective_workdays_override` 会忽略它。
pub fn stamp_override_month(cfg: &mut Config) -> bool {
    if cfg.workdays_override.is_some() && cfg.workdays_override_for.is_none() {
        let now = Local::now();
        cfg.workdays_override_for = Some(format!("{:04}-{:02}", now.year(), now.month()));
        true
    } else {
        false
    }
}

/// 仅当覆盖值属于指定年月时才采用；过期或未设置返回 None。
pub fn effective_workdays_override(cfg: &Config, year: i32, month: u32) -> Option<u32> {
    let n = cfg.workdays_override?;
    let ym = format!("{:04}-{:02}", year, month);
    match cfg.workdays_override_for.as_deref() {
        Some(for_ym) if for_ym == ym => Some(n),
        Some(_) => None,
        None => Some(n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_rejects_invalid_types() {
        let cfg = Config::default();
        let incoming = serde_json::json!({"monthly_salary": "not-a-number"});
        let err = merge_from_value(&cfg, &incoming).unwrap_err();
        assert!(err.contains("合并失败"), "实际: {err}");
    }

    #[test]
    fn merge_keeps_fields_not_in_payload() {
        let mut cfg = Config::default();
        cfg.monthly_salary = 15000.0;
        let incoming = serde_json::json!({"monitor_activity": false});
        let merged = merge_from_value(&cfg, &incoming).unwrap();
        assert_eq!(merged.monthly_salary, 15000.0);
        assert!(!merged.monitor_activity);
    }

    #[test]
    fn override_from_other_month_is_ignored() {
        let mut cfg = Config::default();
        cfg.workdays_override = Some(18);
        cfg.workdays_override_for = Some("2026-10".into());
        assert_eq!(effective_workdays_override(&cfg, 2026, 10), Some(18));
        assert_eq!(
            effective_workdays_override(&cfg, 2026, 11),
            None,
            "10 月的覆盖不得当 11 月时薪分母"
        );
    }

    #[test]
    fn stamp_override_month_only_once() {
        let mut cfg = Config::default();
        cfg.workdays_override = Some(18);
        assert!(stamp_override_month(&mut cfg));
        let stamped = cfg.workdays_override_for.clone();
        assert!(stamped.is_some());
        assert!(!stamp_override_month(&mut cfg), "已有所属月不应再改");
        assert_eq!(cfg.workdays_override_for, stamped);
    }

    // ---- read_config_file：load() 的可测内核，四分类各有归属 ----
    // 用临时目录构造文件，绝不碰真实的 %APPDATA%/niuma-timer/config.json。

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("niuma-cfg-test-{tag}-{}", std::process::id()));
        let _ = fs::create_dir_all(&d);
        d
    }

    fn cleanup(p: &PathBuf, dir: &PathBuf) {
        let _ = fs::remove_file(p);
        let _ = fs::remove_dir_all(dir);
    }

    /// 文件不存在 = Missing（首跑初始化，上层落盘默认值）
    #[test]
    fn read_config_file_missing_is_missing() {
        let p = std::env::temp_dir().join(format!("niuma-cfg-nope-{}.json", std::process::id()));
        assert!(matches!(read_config_file(&p), LoadResult::Missing));
    }

    /// 合法 JSON = Loaded，且缺省字段走 serde default 而非误判损坏。
    /// 注意 6 个无 #[serde(default)] 的核心字段必须齐全（salary/payday/四个时间），
    /// 缺任何一个按 Corrupt 处理——与旧 load() 的 from_str 行为一致。
    #[test]
    fn read_config_file_valid_json_is_loaded() {
        let dir = tmp_dir("valid");
        let p = dir.join("config.json");
        fs::write(
            &p,
            r#"{"monthly_salary": 15000, "payday": 15, "am_start": "09:00", "am_end": "12:00", "pm_start": "13:00", "pm_end": "18:00"}"#,
        )
        .unwrap();
        match read_config_file(&p) {
            LoadResult::Loaded(cfg) => {
                assert_eq!(cfg.monthly_salary, 15000.0);
                assert_eq!(cfg.payday, 15);
                assert_eq!(cfg.duration_format, "hms");
            }
            other => panic!("应 Loaded，实际 {other:?}"),
        }
        cleanup(&p, &dir);
    }

    /// 内容不是合法配置 = Corrupt（上层备份后落盘默认值）
    #[test]
    fn read_config_file_garbage_is_corrupt() {
        let dir = tmp_dir("corrupt");
        let p = dir.join("config.json");
        fs::write(&p, "{ not json !!!").unwrap();
        assert!(matches!(read_config_file(&p), LoadResult::Corrupt));
        cleanup(&p, &dir);
    }

    /// 读失败 = Unreadable（本修复的核心分支）。用「路径指向目录」模拟
    /// 「文件存在但读不了」（Windows 下 read_to_string 对目录报 PermissionDenied，
    /// 区别于 NotFound）——此前 load() 把这种情况当成没配置，直接 save() 默认值
    /// 覆盖磁盘上的真配置，正是要堵的洞。
    #[test]
    fn read_config_file_unreadable_is_unreadable() {
        let dir = tmp_dir("unreadable");
        let p = dir.join("config.json");
        fs::create_dir_all(&p).unwrap();
        assert!(matches!(read_config_file(&p), LoadResult::Unreadable(_)));
        let _ = fs::remove_dir_all(&dir);
    }
}
