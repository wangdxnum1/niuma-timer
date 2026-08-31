use chrono::{DateTime, Datelike, Local, NaiveDate, Timelike};

use crate::config::Config;

/// 当天实时状态
#[derive(Clone, Debug, serde::Serialize)]
pub struct DayStatus {
    pub is_workday: bool,
    /// 时薪（元/小时）
    pub hourly_rate: f64,
    /// 实时赚钱速率（元/分钟）
    pub rate_per_min: f64,
    /// 今天已工作小时
    pub worked_h: f64,
    /// 距下班小时（非工作日或已下班为 0）
    pub to_off_h: f64,
    /// 是否已过下午下班（下班封顶）
    pub off_work: bool,
    /// 当日总工时（上午段+下午段）
    pub daily_hours: f64,
    /// 今天已赚（元）
    pub earned: f64,
    /// 距发薪日天数
    pub days_to_pay: i64,
    /// 已工作时长（格式化字符串，如 "5小时10分30秒"）
    pub worked_str: String,
    /// 距下班时长（格式化字符串；已下班="已下班"，休息日="今天休息"）
    pub to_off_str: String,
    /// 托盘悬停文本
    pub tooltip: String,
    /// 托盘图标文字（如 ¥328）
    pub icon_text: String,
}

/// 时长格式化（分钟 → 人可读文本）
/// hms=几小时几分几秒  hm=几小时几分  h=小数小时
pub fn format_duration(minutes: f64, fmt: &str) -> String {
    let total_secs = (minutes.max(0.0) * 60.0).round() as i64;
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    match fmt {
        "hms" => {
            if h > 0 {
                format!("{}小时{}分{}秒", h, m, s)
            } else if m > 0 {
                format!("{}分{}秒", m, s)
            } else {
                format!("{}秒", s)
            }
        }
        "hm" => {
            if h > 0 {
                format!("{}小时{}分", h, m)
            } else {
                format!("{}分", m)
            }
        }
        _ => format!("{:.1}h", minutes / 60.0),
    }
}

fn to_min(s: &str) -> f64 {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() == 2 {
        if let (Ok(h), Ok(m)) = (parts[0].parse::<f64>(), parts[1].parse::<f64>()) {
            return h * 60.0 + m;
        }
    }
    if let Ok(h) = s.parse::<f64>() {
        return h * 60.0;
    }
    0.0
}

/// 落在 [s, e] 时段内的分钟数（now 为当前分钟，含小数）
fn overlap(now: f64, s: f64, e: f64) -> f64 {
    if now <= s {
        0.0
    } else if now >= e {
        e - s
    } else {
        now - s
    }
}

/// 当日总工时（小时）
pub fn daily_hours(cfg: &Config) -> f64 {
    let am = to_min(&cfg.am_end) - to_min(&cfg.am_start);
    let pm = to_min(&cfg.pm_end) - to_min(&cfg.pm_start);
    ((am + pm).max(0.0)) / 60.0
}

fn days_in_month(y: i32, m: u32) -> u32 {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    NaiveDate::from_ymd_opt(ny, nm, 1)
        .unwrap()
        .pred_opt()
        .expect("valid date")
        .day()
}

/// 距下一个发薪日天数（今天=发薪日则 0）
pub fn days_to_payday(today: NaiveDate, payday: u32) -> i64 {
    let dim = days_in_month(today.year(), today.month());
    let pd = payday.clamp(1, dim);
    let this_month = today.with_day(pd).unwrap();
    let next = if this_month >= today {
        this_month
    } else {
        let (ny, nm) = if today.month() == 12 {
            (today.year() + 1, 1)
        } else {
            (today.year(), today.month() + 1)
        };
        let dim2 = days_in_month(ny, nm);
        let pd2 = payday.clamp(1, dim2);
        NaiveDate::from_ymd_opt(ny, nm, pd2).unwrap()
    };
    (next - today).num_days()
}

/// 计算当天状态
pub fn compute(
    cfg: &Config,
    is_workday: bool,
    monthly_workdays: u32,
    now: DateTime<Local>,
) -> DayStatus {
    let daily_h = daily_hours(cfg);
    let hourly_rate = if monthly_workdays > 0 && daily_h > 0.0 {
        cfg.monthly_salary / (monthly_workdays as f64 * daily_h)
    } else {
        0.0
    };
    let rate_per_min = hourly_rate / 60.0;

    let now_min = now.hour() as f64 * 60.0 + now.minute() as f64 + now.second() as f64 / 60.0;
    let am_s = to_min(&cfg.am_start);
    let am_e = to_min(&cfg.am_end);
    let pm_s = to_min(&cfg.pm_start);
    let pm_e = to_min(&cfg.pm_end);

    let worked_min = overlap(now_min, am_s, am_e) + overlap(now_min, pm_s, pm_e);
    let worked_h = worked_min / 60.0;

    let off_work = is_workday && now_min >= pm_e;
    let to_off_h = if !is_workday {
        0.0
    } else if now_min >= pm_e {
        0.0
    } else {
        (pm_e - now_min) / 60.0
    };

    let earned = worked_h * hourly_rate;
    let days_to_pay = days_to_payday(now.date_naive(), cfg.payday);

    let fmt = &cfg.duration_format;
    let worked_str = format_duration(worked_min, fmt);
    let to_off_str = if !is_workday {
        "今天休息".into()
    } else if now_min >= pm_e {
        "已下班".into()
    } else {
        format_duration(pm_e - now_min, fmt)
    };

    let tooltip = if is_workday {
        // 标签列用全角空格对齐：全角空格宽度严格=1个汉字（半角空格在微软雅黑下≠0.5汉字，会错位）
        // 标签统一 4 字宽 + 1 全角空格，数值起点严格在第 6 列
        format!(
            "今日已赚　¥{:.2}\n已工作　　{}\n距下班　　{}\n────────\n赚钱速率　¥{:.2}/分\n距发薪　　{}天",
            earned, worked_str, to_off_str, rate_per_min, days_to_pay
        )
    } else {
        format!(
            "今天休息\n赚钱速率　¥{:.2}/分\n距发薪　　{}天",
            rate_per_min, days_to_pay
        )
    };

    let icon_text = format!("¥{}", earned.round() as i64);

    DayStatus {
        is_workday,
        hourly_rate,
        rate_per_min,
        worked_h,
        to_off_h,
        off_work,
        daily_hours: daily_h,
        earned,
        days_to_pay,
        worked_str,
        to_off_str,
        tooltip,
        icon_text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, NaiveDate, NaiveTime, TimeZone};

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn format_duration_variants() {
        assert_eq!(format_duration(0.0, "hms"), "0秒");
        assert_eq!(format_duration(60.0, "hms"), "1小时0分0秒");
        assert_eq!(format_duration(65.0, "hms"), "1小时5分0秒");
        assert_eq!(format_duration(5.0, "hms"), "5分0秒");
        assert_eq!(format_duration(5.5, "hms"), "5分30秒");
        assert_eq!(format_duration(60.0, "hm"), "1小时0分");
        assert_eq!(format_duration(5.0, "hm"), "5分");
        assert_eq!(format_duration(60.0, "x"), "1.0h");
        assert_eq!(format_duration(-10.0, "hms"), "0秒");
    }

    #[test]
    fn to_min_variants() {
        assert!(approx(to_min("09:00"), 540.0));
        assert!(approx(to_min("18:30"), 1110.0));
        assert!(approx(to_min("9"), 540.0));
        assert!(approx(to_min("abc"), 0.0));
        assert!(approx(to_min(""), 0.0));
        assert!(approx(to_min("9:"), 0.0));
    }

    #[test]
    fn overlap_variants() {
        assert!(approx(overlap(100.0, 0.0, 200.0), 100.0));
        assert!(approx(overlap(0.0, 0.0, 200.0), 0.0));
        assert!(approx(overlap(300.0, 0.0, 200.0), 200.0));
        assert!(approx(overlap(50.0, 100.0, 200.0), 0.0));
        assert!(approx(overlap(150.0, 100.0, 200.0), 50.0));
    }

    #[test]
    fn daily_hours_default_and_custom() {
        assert!(approx(daily_hours(&Config::default()), 8.0));
        let mut cfg = Config::default();
        cfg.am_start = "10:00".into();
        cfg.am_end = "12:00".into();
        cfg.pm_start = "13:00".into();
        cfg.pm_end = "17:30".into();
        assert!(approx(daily_hours(&cfg), 6.5));
    }

    #[test]
    fn days_to_payday_variants() {
        assert_eq!(days_to_payday(NaiveDate::from_ymd_opt(2026, 8, 1).unwrap(), 10), 9);
        assert_eq!(days_to_payday(NaiveDate::from_ymd_opt(2026, 8, 10).unwrap(), 10), 0);
        assert_eq!(days_to_payday(NaiveDate::from_ymd_opt(2026, 8, 15).unwrap(), 10), 26);
        // 发薪日超过当月天数 → 钳到月末
        assert_eq!(days_to_payday(NaiveDate::from_ymd_opt(2026, 2, 15).unwrap(), 31), 13);
        assert_eq!(days_to_payday(NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(), 31), 29);
    }

    #[test]
    fn compute_workday_morning() {
        let cfg = Config::default();
        let date = NaiveDate::from_ymd_opt(2026, 8, 19).unwrap();
        let now = Local
            .from_local_datetime(&date.and_time(NaiveTime::from_hms_opt(10, 0, 0).unwrap()))
            .single()
            .unwrap();
        let st = compute(&cfg, true, 22, now);
        assert!(st.is_workday);
        assert!(!st.off_work);
        assert!(approx(st.worked_h, 1.0));
        assert!(approx(st.to_off_h, 8.0));
        assert!(approx(st.daily_hours, 8.0));
        let hourly = 15000.0 / (22.0 * 8.0);
        assert!(approx(st.hourly_rate, hourly));
        assert!(approx(st.rate_per_min, hourly / 60.0));
        assert!(approx(st.earned, hourly));
        assert_eq!(st.days_to_pay, 22);
        assert_eq!(st.worked_str, "1小时0分0秒");
        assert_eq!(st.to_off_str, "8小时0分0秒");
    }

    #[test]
    fn compute_restday_gating() {
        let cfg = Config::default();
        let date = NaiveDate::from_ymd_opt(2026, 8, 22).unwrap();
        let now = Local
            .from_local_datetime(&date.and_time(NaiveTime::from_hms_opt(10, 0, 0).unwrap()))
            .single()
            .unwrap();
        let st = compute(&cfg, false, 22, now);
        assert!(!st.is_workday);
        assert!(!st.off_work);
        assert!(approx(st.to_off_h, 0.0));
        assert_eq!(st.to_off_str, "今天休息");
    }
}
