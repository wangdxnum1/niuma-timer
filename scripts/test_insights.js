// 数据洞察回归测试（v1.2.0：时段热力 / 多周趋势 / 身体账单 + 远程排除开关）：
// 1) insights.rs：三组结构体、assemble 纯函数 + with_db 入口、口径 SQL（events/clicks/周封顶）
// 2) main.rs：mod 注册 + 三命令与 handler 登记 + get_week_trend 配置快照
// 3) index.html：账单页 4-tab 分段与三视图骨架、设置页远程排除开关
// 4) app.js：4-tab 状态机与懒加载分发、翻周联动、热力 sqrt 归一、手写 SVG 折线、
//    身体账单 96dpi 换算、远程开关三处接线（readCfg/load 回填/change 保存）
// 5) styles.css：热力金阶 / 趋势浮层 / 身体指标卡
// 运行：node scripts/test_insights.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const appSrc = fs.readFileSync(path.join(ROOT, "frontend", "app.js"), "utf8");
const html = fs.readFileSync(path.join(ROOT, "frontend", "index.html"), "utf8");
const css = fs.readFileSync(path.join(ROOT, "frontend", "styles.css"), "utf8");
const rsInsights = fs.readFileSync(path.join(ROOT, "src-tauri", "src", "insights.rs"), "utf8");
const rsMain = fs.readFileSync(path.join(ROOT, "src-tauri", "src", "main.rs"), "utf8");

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

// ---------------------------------------------------------------- 1. insights.rs 聚合核心
has("结构体 HourCell", rsInsights, "pub struct HourCell");
has("结构体 HourHeatmap", rsInsights, "pub struct HourHeatmap");
has("结构体 TrendPoint", rsInsights, "pub struct TrendPoint");
has("结构体 WeekTrend", rsInsights, "pub struct WeekTrend");
has("结构体 BodyDay", rsInsights, "pub struct BodyDay");
has("结构体 BodyBill", rsInsights, "pub struct BodyBill");
has("events 口径常量（滚轮格数不计）", rsInsights, 'const EVENTS_EXPR: &str = "SUM(moves)+SUM(`left`)+SUM(dbl)+SUM(`right`)+SUM(wheel)+SUM(mid)+SUM(xbtn)+SUM(keys)"');
has("中文星期表 WEEKDAYS_CN", rsInsights, 'const WEEKDAYS_CN: [&str; 7]');
has("热力图纯函数 assemble", rsInsights, "pub fn hour_heatmap_assemble(");
has("热力图 with_db 入口", rsInsights, "pub fn hour_heatmap(week_offset: i64)");
has("趋势复用 weekbill::week_bill", rsInsights, "use crate::weekbill::{week_bill, week_start_of};");
has("摸鱼率纯函数 slack_rate_of", rsInsights, "fn slack_rate_of(front: i64, slack: i64)");
has("前台 0 → 摸鱼率 None（断线不画 0）", rsInsights, "fn slack_rate_of");
has("身体账单纯函数 assemble", rsInsights, "pub fn body_bill_assemble(");
has("身体账单 with_db 入口", rsInsights, "pub fn body_bill(week_offset: i64)");
has("未来周封顶 offset.max(0)", rsInsights, "week_offset.max(0)");
has("clicks 口径：双击折算 1 次", rsInsights, "SUM(`left`)-SUM(dbl)+SUM(`right`)+SUM(mid)+SUM(xbtn)");
has("五指标 COALESCE 防空", rsInsights, "COALESCE(SUM(`left`)-SUM(dbl)+SUM(`right`)+SUM(mid)+SUM(xbtn),0)");
has("热力图三源建表引用 act", rsInsights, "CREATE_ACT_HOURLY");
has("热力图三源建表引用 app", rsInsights, "CREATE_APP_USAGE_HOURLY");
has("热力图三源建表引用 audio", rsInsights, "CREATE_AUDIO_USAGE_HOURLY");
has("身体账单恒 7 天", rsInsights, "for i in 0..7");
has("peak_hour 跨天聚合", rsInsights, "peak_hour");
has("单测：热力图三源聚合", rsInsights, "fn heatmap_aggregates_three_sources");
has("单测：peak 跨天取最大", rsInsights, "fn heatmap_peak_across_days");
has("单测：热力图空周", rsInsights, "fn heatmap_empty_week");
has("单测：clicks 口径与五指标", rsInsights, "fn body_clicks_and_totals");
has("单测：身体账单空周", rsInsights, "fn body_empty_week");
has("单测：摸鱼率规则", rsInsights, "fn slack_rate_rules");
has("单测：趋势窗口 8 周", rsInsights, "fn trend_window_is_eight_weeks");
has("单测：未来周封顶", rsInsights, "fn week_bounds_caps_future");
eq("只测纯函数不碰真实文件库（无 with_db 调用进测试）", /mod tests \{[\s\S]*\bwith_db\b/.test(rsInsights), false);

// ---------------------------------------------------------------- 2. main.rs 接线
has("mod 注册 insights", rsMain, "mod insights;");
has("命令 get_hour_heatmap", rsMain, "fn get_hour_heatmap");
has("命令 get_week_trend", rsMain, "fn get_week_trend");
has("命令 get_body_bill", rsMain, "fn get_body_bill");
has("handler 登记 get_hour_heatmap", rsMain, "get_hour_heatmap,");
has("handler 登记 get_week_trend", rsMain, "get_week_trend,");
has("handler 登记 get_body_bill", rsMain, "get_body_bill,");
has("趋势命令锁内取配置快照", rsMain, 'sync::lock(&state.config, "state.config").clone()');

// ---------------------------------------------------------------- 3. index.html 结构
has("洞察翻页器", html, 'id="billPager"');
has("翻页器上一页按钮", html, 'id="billPgPrev"');
has("翻页器页名", html, 'id="billPgName"');
has("翻页器下一页按钮", html, 'id="billPgNext"');
has("tab：本周账单", html, 'data-btab="bill"');
has("tab：时段热力", html, 'data-btab="heat"');
has("tab：趋势", html, 'data-btab="trend"');
has("tab：身体账单", html, 'data-btab="body"');
has("原账单容器归入 tab", html, 'id="billTabBill"');
has("热力容器", html, 'id="billTabHeat"');
has("热力汇总行", html, 'id="heatSummary"');
has("热力列标区", html, 'id="heatHours"');
has("热力格子区", html, 'id="heatGrid"');
has("热力图例", html, 'class="heat-legend"');
has("热力空态", html, 'id="heatEmpty"');
has("趋势容器", html, 'id="billTabTrend"');
has("趋势图例金线", html, 'trend-dot income');
has("趋势图例红线", html, 'trend-dot slack');
has("趋势 SVG", html, 'id="trendSvg"');
has("趋势浮层", html, 'id="trendTip"');
has("趋势 x 轴", html, 'id="trendXAxis"');
has("趋势空态", html, 'id="trendEmpty"');
has("身体容器", html, 'id="billTabBody"');
has("身体指标卡：点击", html, 'id="bodyClicks"');
has("身体指标卡：按键", html, 'id="bodyKeys"');
has("身体指标卡：滑行", html, 'id="bodyPixels"');
has("身体指标卡：滚轮", html, 'id="bodyWheel"');
has("身体最累日", html, 'id="bodyBusiest"');
has("身体按天柱区", html, 'id="bodyBars"');
has("身体口径脚注", html, 'id="bodyFoot"');
has("身体空态", html, 'id="bodyEmpty"');
has("远程排除开关", html, 'id="overtime_exclude_remote"');
has("远程排除 hint", html, "检测到远程桌面会话时暂停自动加班记录");

// ---------------------------------------------------------------- 4. app.js 状态机与三视图
has("翻页循环 billStep", appSrc, "function billStep(delta)");
has("翻页器圆点作用域限定", appSrc, 'querySelectorAll("#billPager .pg-dot")');
has("账单滚轮翻页", appSrc, '$("viewBill").addEventListener(');
has("tab 状态切换 setBillTabUI", appSrc, "function setBillTabUI(tab)");
has("懒加载分发 loadBillTab", appSrc, "async function loadBillTab()");
has("进账单页同步翻页器并拉当前页", appSrc, "setBillTabUI(curBillTab); // 翻页器页名/圆点与记忆的页保持同步");
has("翻周联动当前 tab", appSrc, "loadBillTab(); // 翻周联动当前 tab");
has("invoke get_hour_heatmap", appSrc, 'invoke("get_hour_heatmap", { weekOffset })');
has("invoke get_week_trend", appSrc, 'invoke("get_week_trend", { weekOffset })');
has("invoke get_body_bill", appSrc, 'invoke("get_body_bill", { weekOffset })');
has("热力 sqrt 归一 heatLevel", appSrc, "function heatLevel(v, max)");
has("热力渲染 paintHourHeat", appSrc, "function paintHourHeat()");
has("趋势渲染 paintWeekTrend", appSrc, "function paintWeekTrend()");
has("身体渲染 paintBodyBill", appSrc, "function paintBodyBill()");
has("手写 SVG 用 createElementNS（无外部库）", appSrc, 'createElementNS(SVG_NS, tag)');
has("摸鱼率 null 断线不画 0", appSrc, "p.slack_rate == null");
has("趋势空窗口判定", appSrc, "pts.some((p) => (p.income || 0) > 0 || p.slack_rate != null)");
has("万次缩写 fmtWan", appSrc, "function fmtWan(n)");
has("96dpi 像素换米", appSrc, "function fmtMeters(px)");
has("日期加 n 天工具", appSrc, "function isoAddDays(iso, n)");
has("热力 hover 提示 title", appSrc, "cell.title =");
has("身体最累日文案", appSrc, "最累 ");
has("身体口径脚注文案", appSrc, "点击 = 单击+右键+中键+侧键，双击折算 1 次");
has("身体空态判定按天 events", appSrc, "days.some((d) => (d.events || 0) > 0)");

// 远程排除开关三处接线
has("readCfg 采集远程排除", appSrc, 'overtime_exclude_remote: $("overtime_exclude_remote").checked,');
has("load 回填（缺字段默认 true）", appSrc, "cfg.overtime_exclude_remote !== false");
has("开关变更立即保存", appSrc, '$("overtime_exclude_remote").addEventListener("change", saveNow);');

// ---------------------------------------------------------------- 5. styles.css 关键样式
has("CSS：热力网格 25 列", css, "grid-template-columns: 34px repeat(24, 1fr);");
has("CSS：热力 0 级深灰", css, "#26262a");
has("CSS：热力 1 级", css, "#4d3f14");
has("CSS：热力 2 级", css, "#8a6d1a");
has("CSS：热力 3 级", css, "#c7a227");
has("CSS：热力 4 级金", css, "#ffd650");
has("CSS：趋势浮层", css, ".trend-tip {");
has("CSS：趋势卡", css, ".trend-wrap {");
has("CSS：身体 2×2 网格", css, ".body-grid {");
has("CSS：身体指标卡", css, ".body-kpi .body-v {");

// ---------------------------------------------------------------- 6. 兜底：$() 引用存在
const refIds = [...appSrc.matchAll(/\$\("([A-Za-z_]\w*)"\)/g)].map((m) => m[1]);
const missing = [...new Set(refIds)].filter((id) => !html.includes('id="' + id + '"'));
eq("app.js 引用的元素全部存在于 index.html", JSON.stringify(missing), "[]");

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
