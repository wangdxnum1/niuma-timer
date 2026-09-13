// 本周打工账单回归测试（小票/仪表盘双风格周聚合视图）：
// 1) weekbill.rs：WeekBill/DayBill 结构体、周一对齐、环比除零、聚合 SQL（ot/act/app）
// 2) main.rs：mod 注册 + get_week_bill 命令与 handler 登记
// 3) config.rs：bill_style 字段 + serde 默认 receipt
// 4) index.html：侧栏第四 tab、viewBill 骨架（周导航/空态/两风格容器）、设置风格分段
// 5) app.js：invoke get_week_bill、本周封顶、四档金句、双风格渲染、监控关闭守卫、
//    设置绑定、mon-seg 类名冲突守卫
// 运行：node scripts/test_week_bill.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const appSrc = fs.readFileSync(path.join(ROOT, "frontend", "app.js"), "utf8");
const html = fs.readFileSync(path.join(ROOT, "frontend", "index.html"), "utf8");
const css = fs.readFileSync(path.join(ROOT, "frontend", "styles.css"), "utf8");
const rsBill = fs.readFileSync(path.join(ROOT, "src-tauri", "src", "weekbill.rs"), "utf8");
const rsMain = fs.readFileSync(path.join(ROOT, "src-tauri", "src", "main.rs"), "utf8");
const rsConfig = fs.readFileSync(path.join(ROOT, "src-tauri", "src", "config.rs"), "utf8");

let pass = 0;
let fail = 0;
function eq(label, actual, expect) {
  if (actual === expect) {
    pass++;
    console.log("  PASS " + label + "  ->  " + actual);
  } else {
    fail++;
    console.log("  FAIL " + label + "  ->  " + actual + "  (期望 " + expect + ")");
  }
}
function has(label, src, needle) {
  eq(label, src.includes(needle), true);
}

// ---------------------------------------------------------------- 1. weekbill.rs 聚合核心
has("结构体 WeekBill", rsBill, "pub struct WeekBill");
has("结构体 DayBill", rsBill, "pub struct DayBill");
has("结构体 WeekInput", rsBill, "pub struct WeekInput");
has("周一对齐 week_start_of", rsBill, "pub fn week_start_of");
has("环比除零返 None", rsBill, "pub fn delta_pct");
has("跨年周降级 builtin_cache", rsBill, "builtin_cache(");
has("满勤=月薪÷当月工作日数", rsBill, "fn monthly_workdays_of");
has("聚合入口 assemble", rsBill, "pub fn assemble");
has("对外命令入口 week_bill", rsBill, "pub fn week_bill");
has("未来周封顶 offset.max(0)", rsBill, "week_offset.max(0)");
// 三条聚合 SQL
has("加班费区间求和", rsBill, "FROM ot_records WHERE date >= ?1 AND date < ?2");
has("键鼠事件聚合", rsBill, "FROM act_hourly WHERE date >= ?1 AND date < ?2");
has("点击口径化简式（(left−2×dbl)+dbl 合并）", rsBill, "SUM(`left`)-SUM(dbl)+SUM(`right`)");
has("应用秒数按天按应用聚合", rsBill, "FROM app_usage WHERE date >= ?1 AND date < ?2 GROUP BY date, app");
has("摸鱼分类复用 category_of", rsBill, "app_usage::category_of(&app, input.cfg)");
// 单测覆盖
has("单测：周一对齐", rsBill, "fn week_start_alignment");
has("单测：跨月周工资分母", rsBill, "fn cross_month_full_salary");
has("单测：今日实时未来零", rsBill, "fn today_realtime_future_zero");
has("单测：休息日跳过工资", rsBill, "fn rest_day_salary_zero_but_ot_recorded");
has("单测：监控全关 work_days 为 null", rsBill, "fn monitors_off_work_days_null");
has("单测：空周全零", rsBill, "fn empty_week_all_zero");

// ---------------------------------------------------------------- 2. main.rs 接线
has("mod 注册 weekbill", rsMain, "mod weekbill;");
has("命令 get_week_bill", rsMain, "fn get_week_bill");
has("handler 登记 get_week_bill", rsMain, "get_week_bill,");
has("命令锁外查询：clone 配置快照", rsMain, "sync::lock(&state.config, \"state.config\").clone()");

// ---------------------------------------------------------------- 3. config.rs bill_style
has("Config 新增 bill_style 字段", rsConfig, "pub bill_style: String");
has("serde 默认 default_bill_style", rsConfig, 'default = "default_bill_style"');
has("默认值函数 receipt", rsConfig, "fn default_bill_style() -> String {");
has("默认值 receipt", rsConfig, '"receipt".into()');
has("Default impl 含 bill_style", rsConfig, 'bill_style: "receipt".into(),');

// ---------------------------------------------------------------- 4. index.html 结构
has("侧栏第四 tab viewBill", html, 'data-nav="viewBill"');
has("侧栏账单文案", html, ">账单<");
has("周导航上一周按钮", html, 'id="billPrevWeek"');
has("周导航标签", html, 'id="billWeekLabel"');
has("周导航下一周按钮", html, 'id="billNextWeek"');
has("空态容器", html, 'id="billEmpty"');
has("小票容器", html, 'id="billReceipt"');
has("小票编号", html, 'id="rcpNo"');
has("小票金句", html, 'id="rcpQuote"');
has("小票 7 日金条区", html, 'id="rcpDays"');
has("红章已验讫", html, "已验讫");
has("仪表盘容器", html, 'id="billDash"');
has("KPI 金额", html, 'id="kpiAmount"');
has("环比箭头", html, 'id="kpiDelta"');
has("指标格 2×2", html, 'class="mini-grid"');
has("最累最摸行", html, 'id="dashExtreme"');
has("每日双柱区", html, 'id="dashBars"');
has("设置风格分段容器", html, 'id="billStyleSeg"');
has("分段：小票", html, 'data-bill="receipt"');
has("分段：仪表盘", html, 'data-bill="dashboard"');

// ---------------------------------------------------------------- 5. app.js 渲染与守卫
has("invoke get_week_bill", appSrc, 'invoke("get_week_bill", { weekOffset })');
has("懒渲染：进账单页才拉", appSrc, 'if (id === "viewBill") loadWeekBill();');
has("翻周状态 weekOffset", appSrc, "let weekOffset = 0;");
has("本周封顶", appSrc, "if (next < 0) return;");
has("四档金句表 WEEK_BILL_QUIPS", appSrc, "const WEEK_BILL_QUIPS");
has("金句函数 weekBillQuip", appSrc, "function weekBillQuip(ratePct, withMoney)");
has("金句：<10%", appSrc, "本周天选牛马，老板的战略合作伙伴");
has("金句：10-25%", appSrc, "摸得克制，装得敬业");
has("金句：25-40%", appSrc, "将近三分之一的班，上给了手机");
has("金句：≥40%", appSrc, "本周工资建议原路退回");
has("渲染总入口 paintWeekBill", appSrc, "function paintWeekBill()");
has("本周禁用下一周按钮", appSrc, '$("billNextWeek").disabled = !!bill.is_current_week;');
has("空态判定按 has_record", appSrc, "anyRecord");
has("小票渲染 paintReceipt", appSrc, "function paintReceipt(bill)");
has("仪表盘渲染 paintDash", appSrc, "function paintDash(bill)");
has("金句门槛 front_seconds>0", appSrc, "if (front > 0)");
has("守卫：未配月薪隐藏工资摸鱼", appSrc, "function moneyConfigured()");
has("守卫：监控关显示 —", appSrc, 'bill.work_days == null ? "—"');
has("环比无基数不显示箭头", appSrc, "bill.delta_pct == null");
has("双柱各按自身最大值归一", appSrc, "const maxSlack");
has("今日实时态", appSrc, "const today = todayStr();");
has("设置读风格 readBillStyle", appSrc, "function readBillStyle()");
has("设置写风格 setBillStyleUI", appSrc, "function setBillStyleUI(style)");
has("load 时应用已存风格", appSrc, 'setBillStyleUI(cfg.bill_style || "receipt");');
has("readCfg 采集 bill_style", appSrc, "bill_style: readBillStyle(),");
has("风格切换保存", appSrc, "setBillStyleUI(b.dataset.bill);");
has("切风格用缓存重画", appSrc, "paintWeekBill();");
// 监控预览分段与账单分段复用 .mon-seg-item 的类名冲突守卫
has("mon-seg 冲突守卫", appSrc, "if (!btn.dataset.mon) return;");

// ---------------------------------------------------------------- 6. styles.css 关键样式
has("CSS：小票票面", css, ".receipt {");
has("CSS：锯齿边", css, ".receipt::after {");
has("CSS：红章", css, ".rcp-stamp {");
has("CSS：金条", css, ".rcp-day-fill {");
has("CSS：空态", css, ".bill-empty {");
has("CSS：KPI 金额", css, ".kpi-val {");
has("CSS：入账金柱", css, ".bar-earn {");
has("CSS：摸鱼红柱", css, ".bar-slack {");

// ---------------------------------------------------------------- 7. 兜底：$() 引用存在
const refIds = [...appSrc.matchAll(/\$\("([A-Za-z_]\w*)"\)/g)].map((m) => m[1]);
const missing = [...new Set(refIds)].filter((id) => !html.includes('id="' + id + '"'));
eq("app.js 引用的元素全部存在于 index.html", JSON.stringify(missing), "[]");

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
