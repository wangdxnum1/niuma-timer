// B 阶段账单泛化断言脚本（零注册：run_all.js 自动收集 scripts/test_*.js）
// 用法：node scripts/test_bill.js —— 退出码 0 = 全绿，1 = 有失败
const fs = require("fs");
const path = require("path");

const root = path.join(__dirname, "..");
const read = (p) => fs.readFileSync(path.join(root, p), "utf8");

let pass = 0, fail = 0;
const eq = (name, a, b) => {
  if (a === b) { pass++; } else { fail++; console.log("FAIL " + name + "  期望 " + JSON.stringify(b) + " 实际 " + JSON.stringify(a)); }
};
const has = (name, hay, needle) => eq(name, hay.includes(needle), true);
const lacks = (name, hay, needle) => eq(name, hay.includes(needle), false);

const rsConfig = read("src-tauri/src/config.rs");
const rsBill = read("src-tauri/src/weekbill.rs");
const rsInsights = read("src-tauri/src/insights.rs");
const rsRemind = read("src-tauri/src/remind.rs");
const rsMain = read("src-tauri/src/main.rs");
const html = read("frontend/index.html");
const appSrc = read("frontend/app.js");
const caps = read("src-tauri/capabilities/default.json");

// ---- 区块 1：配置字段（B1）----
has("config 有 bill_span 字段", rsConfig, "pub bill_span: String,");
has("config 有 bill_span serde 默认", rsConfig, '#[serde(default = "default_bill_span")]');
has("config 有 default_bill_span 函数", rsConfig, "fn default_bill_span() -> String {");
has("config Default 有 bill_span 初始", rsConfig, 'bill_span: "week".into(),');
has("config 有 bill_span 注释", rsConfig, '/// 账单视图上次选择的跨度："week" / "month" / "year"');
has("config 有 remind_payday_enabled 字段", rsConfig, "pub remind_payday_enabled: bool,");
has("config Default 有 remind_payday 初始", rsConfig, "remind_payday_enabled: true,");
has("config 有发薪日注释", rsConfig, "/// 发薪日战绩提醒开关");
has("config merge 零登记说明", rsConfig, "新增字段零登记");

// ---- 区块 2：Span 枚举与周期区间（B2）----
has("weekbill 有 Span 枚举", rsBill, "pub enum Span { Week, Month, Year }");
has("weekbill 有 parse_span", rsBill, "pub fn parse_span(s: &str) -> Result<Span, String> {");
has("weekbill 未知跨度显式报错", rsBill, 'other => Err(format!("未知账单跨度: {other}"))');
has("weekbill 有 period_bounds", rsBill, "pub fn period_bounds(span: Span, offset: i64, today: NaiveDate) -> (NaiveDate, NaiveDate)");
has("weekbill 有 prev_period_bounds", rsBill, "pub fn prev_period_bounds(span: Span, offset: i64, today: NaiveDate) -> (NaiveDate, NaiveDate)");
has("weekbill 有 last_day_of", rsBill, "fn last_day_of(year: i32, month: u32) -> u32 {");
has("单测 周区间与旧 week_bounds 等价", rsBill, "fn period_bounds_week_matches_old_week_bounds()");
has("单测 月年区间含闰年", rsBill, "fn period_bounds_month_year()");
has("单测 上一周期即偏移加一", rsBill, "fn prev_period_bounds_is_offset_plus_one()");
has("单测 parse_span 拒绝未知", rsBill, "fn parse_span_rejects_unknown()");

// ---- 区块 3：PeriodBill 泛化（B3）----
has("weekbill 有 PeriodInput", rsBill, "pub struct PeriodInput<'a> {");
has("weekbill 有 PeriodBill", rsBill, "pub struct PeriodBill {");
has("PeriodBill 有 period_label", rsBill, "pub period_label: String,");
has("PeriodBill 有 is_current_period", rsBill, "pub is_current_period: bool,");
has("PeriodBill 有 ot_hours", rsBill, "pub ot_hours: f64,");
has("weekbill 有 period_bill 入口", rsBill, "pub fn period_bill(cfg: &Config, cur_hol: &HolidayCache, span: Span, offset: i64) -> Result<PeriodBill, String>");
has("weekbill 有 period_label_of", rsBill, "fn period_label_of(");
has("assemble 用 while 遍历区间", rsBill, "while d <= input.end");
has("period_bill 复用 period_bounds", rsBill, "period_bounds(span, off, today)");
has("period_bill 环比取上一周期", rsBill, "prev_period_bounds(span, off, today)");
has("main 命令返回 PeriodBill", rsMain, "Result<weekbill::PeriodBill, String>");
has("main 命令体调 period_bill", rsMain, "weekbill::period_bill(&cfg, &hol, s, offset)");
has("insights 复用 period_bill", rsInsights, "period_bill(cfg, cur_hol, span, off + i)");
has("insights 取 period_start", rsInsights, "period_start: pb.period_start,");
lacks("旧 WeekBill 已移除", rsBill, "pub struct WeekBill");
lacks("旧 WeekInput 已移除", rsBill, "pub struct WeekInput");

// ---- 区块 4：分桶账单（B4）----
has("weekbill 有 BucketBill", rsBill, "pub struct BucketBill {");
has("BucketBill date 可空", rsBill, "pub date: Option<String>,");
has("BucketBill is_workday 可空", rsBill, "pub is_workday: Option<bool>,");
has("weekbill 有 bucketize", rsBill, "pub fn bucketize(days: &[DayBill], span: Span) -> Vec<BucketBill>");
has("weekbill 有 day_to_bucket", rsBill, "fn day_to_bucket(d: &DayBill) -> BucketBill {");
has("weekbill 有 month_bucket", rsBill, "fn month_bucket(label: String, ds: Vec<&DayBill>) -> BucketBill {");
has("年桶 12 恒项", rsBill, "1u32..=12");
has("DayBill 有 front_seconds", rsBill, "pub front_seconds: i64,");
has("PeriodBill 已无 days 字段", rsBill, "pub buckets: Vec<BucketBill>,");
lacks("旧 days 字段声明已移除", rsBill, "pub days: Vec<DayBill>,");

// ---- 区块 5：insights 泛化（B5）----
has("HourCell 有 weekday 字段", rsInsights, "pub weekday: String");
has("insights 有 hour_heatmap 泛化入口", rsInsights, "pub fn hour_heatmap(span: Span, offset: i64) -> Result<HourHeatmap, String>");
has("insights 有 body_bill 泛化入口", rsInsights, "pub fn body_bill(span: Span, offset: i64) -> Result<BodyBill, String>");
has("insights 有 period_trend", rsInsights, "pub fn period_trend(cfg: &Config, cur_hol: &HolidayCache, span: Span, offset: i64) -> Result<WeekTrend, String>");
has("热力月年坍缩函数", rsInsights, "fn collapse_to_weekly_cells(hm: HourHeatmap) -> HourHeatmap");
has("身体年坍缩函数", rsInsights, "fn collapse_days_to_months(bb: BodyBill) -> BodyBill");
has("TrendPoint 有 period_end", rsInsights, "pub period_end: String,");
has("TrendPoint 有 label", rsInsights, "pub label: String,");
has("趋势复用 Span", rsInsights, "Span::Week => hm");
lacks("旧 week_bounds 已删除", rsInsights, "fn week_bounds");
lacks("旧 week_bill import 已移除", rsInsights, "week_bill,");

// ---- 区块 6：命令改名与 span 入参（B6）----
has("main 有 get_bill 命令", rsMain, "fn get_bill(state: State<'_, AppState>, span: String, offset: i64)");
has("main 有 get_heatmap 命令", rsMain, "fn get_heatmap(span: String, offset: i64)");
has("main 有 get_trend 命令", rsMain, "fn get_trend(state: State<'_, AppState>, span: String, offset: i64)");
has("main 有 get_body_bill 命令", rsMain, "fn get_body_bill(span: String, offset: i64)");
has("get_bill 先解析 span 再取锁", rsMain, "let s = weekbill::parse_span(&span)?;");
has("注册表有 get_bill", rsMain, "get_bill,");
has("注册表有 get_heatmap", rsMain, "get_heatmap,");
has("注册表有 get_trend", rsMain, "get_trend,");
lacks("旧 get_week_bill 已移除", rsMain, "get_week_bill");
lacks("旧 get_hour_heatmap 已移除", rsMain, "get_hour_heatmap");
lacks("旧 get_week_trend 已移除", rsMain, "get_week_trend");
lacks("capabilities 陈旧 allow-get-week-bill 已删", caps, "allow-get-week-bill");
lacks("capabilities 陈旧 allow-get-hour-heatmap 已删", caps, "allow-get-hour-heatmap");
lacks("capabilities 陈旧 allow-get-week-trend 已删", caps, "allow-get-week-trend");

// ---- 区块 7：发薪日通知（B7）----
has("remind 有 PAYDAY_DONE", rsRemind, "static PAYDAY_DONE: AtomicBool");
has("remind 有 PAYDAY_DATE", rsRemind, "static PAYDAY_DATE: Mutex<Option<chrono::NaiveDate>>");
has("remind 有 payday_tick", rsRemind, "fn payday_tick(app: &tauri::AppHandle, cfg: &Config) {");
has("tick 已接线 payday_tick", rsRemind, "payday_tick(app, &cfg);");
has("播报上月账单", rsRemind, "Span::Month, 1");
has("remind 有判定纯函数", rsRemind, "pub fn should_notify_payday(enabled: bool, today_day: u32, payday: u32, done: bool) -> bool");
has("remind 有千分位", rsRemind, "fn thousands(n: i64) -> String {");
has("remind 有文案函数", rsRemind, "pub(crate) fn payday_texts(");
has("单测 命中与未命中", rsRemind, "fn payday_hit_and_miss()");
has("单测 千分位", rsRemind, "fn thousands_format()");
has("单测 跨年上月", rsRemind, "fn payday_prev_month_crosses_year()");
has("remind use 已扩 Datelike", rsRemind, "use chrono::{Datelike, Timelike};");

// ---- 区块 8：前端接线（B8）----
has("html 有跨度切换器", html, 'id="billSpanSeg"');
has("html 有周按钮", html, 'data-span="week"');
has("html 有月按钮", html, 'data-span="month"');
has("html 有年按钮", html, 'data-span="year"');
has("html 有发薪日开关", html, 'id="remind_payday_enabled"');
has("文案 上一期", html, "‹ 上一期");
has("文案 下一期", html, "下一期 ›");
has("文案 这一期还没有打工记录", html, "这一期还没有打工记录");
has("文案 本期打工账单", html, "本期打工账单");
has("文案 本期总入账", html, "本期总入账");
has("app 有 curBillSpan", appSrc, 'let curBillSpan = "week";');
has("app 有 readBillSpan", appSrc, "function readBillSpan() {");
has("app 有 setBillSpanUI", appSrc, "function setBillSpanUI(span) {");
has("app get_bill 带 span", appSrc, 'invoke("get_bill", { span: curBillSpan, offset: weekOffset })');
has("app get_heatmap 带 span", appSrc, 'invoke("get_heatmap", { span: curBillSpan, offset: weekOffset })');
has("app get_trend 带 span", appSrc, 'invoke("get_trend", { span: curBillSpan, offset: weekOffset })');
has("app get_body_bill 带 span", appSrc, 'invoke("get_body_bill", { span: curBillSpan, offset: weekOffset })');
has("app 渲染用 buckets", appSrc, "(bill.buckets || [])");
has("app 用 is_current_period", appSrc, "!!bill.is_current_period");
has("app 趋势 nav 用 period 字段", appSrc, "paintBillNav(last.period_start, last.period_end)");
has("app 热力按星期聚合取格", appSrc, 'c.weekday + " " + c.hour');
has("app 热力行标签 getDay 修正", appSrc, "WD_CN[(i + 1) % 7]");
has("app CSV 按期导出", appSrc, '"账单_" + (bill.period_start || "") + ".csv"');
has("app readCfg 记忆 bill_span", appSrc, "bill_span: readBillSpan(),");
has("app load 恢复 bill_span", appSrc, 'setBillSpanUI(cfg.bill_span || "week");');
has("app 切跨度重置偏移", appSrc, "weekOffset = 0; // 切跨度重置偏移：上一期的语义随跨度变化");
lacks("旧 isoAddDays 已删除", appSrc, "function isoAddDays(");

console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
