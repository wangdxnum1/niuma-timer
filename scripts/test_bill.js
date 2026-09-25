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

console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
