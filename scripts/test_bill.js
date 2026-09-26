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
has("main 命令体调 period_bill", rsMain, "weekbill::period_bill(&cfg, &hol, weekbill::Span::Week, week_offset)");
has("insights 复用 period_bill", rsInsights, "period_bill(cfg, cur_hol, Span::Week, off + i)");
has("insights 取 period_start", rsInsights, "week_start: wb.period_start,");
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

console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
