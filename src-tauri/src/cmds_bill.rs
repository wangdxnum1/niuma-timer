//! 加班与账单命令：加班明细增删改查、周/月/年账单聚合、数据洞察三视图与时间线回顾。
//!
//! 二期自 main.rs 切出（纯移动）。

use chrono::{Datelike, Duration, Local};
use tauri::State;

use crate::db;
use crate::focus;
use crate::insights;
use crate::overtime;
use crate::state::AppState;
use crate::sync;
use crate::weekbill;

/// 获取指定月份的加班记录（含预计算汇总字段）。
/// year / month 省略时取当前月——老调用方不传参也能正常工作。
/// async：查询走全局 DB 锁，「立即整理」持锁秒级时不得冻结主线程（同 get_bill）。
#[tauri::command(async)]
pub(crate) fn get_overtime_records(
    year: Option<i32>,
    month: Option<u32>,
) -> overtime::MonthlyOvertimeView {
    let now = Local::now();
    let y = year.unwrap_or_else(|| now.year());
    let m = month.unwrap_or_else(|| now.month());
    overtime::get_month(y, m).to_view(y, m)
}

/// 手动添加/修改某天加班记录（可补录历史月份，但不能是未来日期）。
/// 返回该记录**所属月份**的视图：补录 8 月时界面不会莫名跳回当月。
#[tauri::command(async)]
pub(crate) fn save_overtime_record(
    state: State<AppState>,
    input: overtime::ManualOvertimeInput,
) -> Result<overtime::MonthlyOvertimeView, String> {
    let now = Local::now();
    let (y, m) = overtime::month_of(&input.date).unwrap_or_else(|| (now.year(), now.month()));
    let cfg = sync::lock(&state.config, "state.config").clone();
    let hol = sync::lock(&state.holiday, "state.holiday").clone();
    overtime::save_manual(input, &cfg, Some(&hol))?;
    Ok(overtime::get_month(y, m).to_view(y, m))
}

/// 手动删除某天加班记录（历史月份同样可删）。返回该记录**所属月份**的视图。
#[tauri::command(async)]
pub(crate) fn delete_overtime_record(
    date: String,
) -> Result<overtime::MonthlyOvertimeView, String> {
    let now = Local::now();
    let (y, m) = overtime::month_of(&date).unwrap_or_else(|| (now.year(), now.month()));
    overtime::delete_manual(&date)?;
    Ok(overtime::get_month(y, m).to_view(y, m))
}

/// 账单聚合（span = week/month/year；offset：0=本期，正数往前翻历史期，负数封顶本期）
///
/// 锁内只取 config/holiday 快照，DB 查询全部在锁外（ABBA 死锁规避，见 weekbill 模块注释）。
#[tauri::command(async)]
pub(crate) fn get_bill(
    state: State<'_, AppState>,
    span: String,
    offset: i64,
) -> Result<weekbill::PeriodBill, String> {
    let s = weekbill::parse_span(&span)?;
    let cfg = sync::lock(&state.config, "state.config").clone();
    let hol = sync::lock(&state.holiday, "state.holiday").clone();
    weekbill::period_bill(&cfg, &hol, s, offset)
}

/// 时段热力图（7×24 键鼠/前台/音频小时代格；月/年坍缩为星期×小时矩阵）。纯 act/app/audio 表聚合，无需快照。
#[tauri::command(async)]
pub(crate) fn get_heatmap(span: String, offset: i64) -> Result<insights::HourHeatmap, String> {
    let s = weekbill::parse_span(&span)?;
    insights::hour_heatmap(s, offset)
}

/// 多周期趋势（最近 8 个周期，入账复用账单口径）——快照模式同 get_bill。
#[tauri::command(async)]
pub(crate) fn get_trend(
    state: State<'_, AppState>,
    span: String,
    offset: i64,
) -> Result<insights::WeekTrend, String> {
    let s = weekbill::parse_span(&span)?;
    let cfg = sync::lock(&state.config, "state.config").clone();
    let hol = sync::lock(&state.holiday, "state.holiday").clone();
    insights::period_trend(&cfg, &hol, s, offset)
}

/// 身体账单（键鼠损耗五指标 + 按期分布）。纯 act_hourly 聚合，无需快照。
#[tauri::command(async)]
pub(crate) fn get_body_bill(span: String, offset: i64) -> Result<insights::BodyBill, String> {
    let s = weekbill::parse_span(&span)?;
    insights::body_bill(s, offset)
}

/// 时间线回顾（v1.6.0）：某天 24 小时的前台构成 / 键鼠 / 媒体。
/// offset 0=今天，正数往过去翻，未来封顶今天（与账单 offset 同向语义）。
/// 锁内取 config/holiday 快照，DB 查询在锁外（同 get_bill 的 ABBA 规避）。
#[tauri::command(async)]
pub(crate) fn get_day_timeline(
    state: State<'_, AppState>,
    offset: i64,
) -> Result<insights::DayTimeline, String> {
    // offset 是前端 IPC 直达的 i64：只挡负值不够，极端大值会让 chrono 日期运算
    // panic（异步命令 panic 被任务边界吞掉、前端 invoke 永不 resolve）。与
    // weekbill::period_bounds 同口径钳制——时间线按天翻，1200 天封顶。
    let off = offset.clamp(0, 1200);
    let today = Local::now().date_naive();
    let date = today - Duration::days(off);
    let cfg = sync::lock(&state.config, "state.config").clone();
    let hol = sync::lock(&state.holiday, "state.holiday").clone();
    db::with_db(|conn| insights::day_timeline_assemble(date, &cfg, &hol, conn))
        .map_err(|e| e.to_string())
}

/// 专注段聚合（v1.7.0）：周期内每日段数/总时长/最长一段。快照模式同 get_bill。
#[tauri::command(async)]
pub(crate) fn get_focus_summary(
    state: State<'_, AppState>,
    span: String,
    offset: i64,
) -> Result<focus::FocusSummary, String> {
    let s = weekbill::parse_span(&span)?;
    let hol = sync::lock(&state.holiday, "state.holiday").clone();
    focus::period_summary(&hol, s, offset)
}
