// 时间线回顾（v1.6.0）契约测试：
//  1) Rust：聚合函数与命令（offset 未来封顶、查询时归类、24 行固定）
//  2) 注册链：generate_handler 登记 + capabilities allow-*
//  3) 前端：tab 接入（pane/圆点/分发）+ 日导航封顶 + 渲染结构（四色构成条）
//  4) 口径：无前台记录不臆造「离开」（空小时全 0），监控关闭 ≠ 离开
//
// 运行：node scripts/test_day_timeline.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const read = (p) => fs.readFileSync(path.join(ROOT, p), "utf-8").replace(/\r\n/g, "\n");
const rsInsights = read("src-tauri/src/insights.rs");
const rsBill = read("src-tauri/src/cmds_bill.rs");
const rsMain = read("src-tauri/src/main.rs");
const caps = read("src-tauri/capabilities/default.json");
const html = read("frontend/index.html");
const { rsSource } = require("./lib/rs_sources");
const rsSrc = rsSource().replace(/\r\n/g, "\n");
const { feSource } = require("./lib/fe_sources");
const appSrc = feSource().replace(/\r\n/g, "\n");

let pass = 0, fail = 0;
const eq = (name, a, b) => {
  if (a === b) { pass++; } else { fail++; console.log("FAIL " + name + "  期望 " + JSON.stringify(b) + " 实际 " + JSON.stringify(a)); }
};
const ok = (name, cond) => eq(name, !!cond, true);
const has = (name, hay, needle) => eq(name, typeof hay === "string" && hay.includes(needle), true);
const lacks = (name, hay, needle) => eq(name, !(typeof hay === "string" && hay.includes(needle)), true);

console.log("== Rust 聚合（insights.rs） ==");
has("聚合纯函数 day_timeline_assemble", rsInsights, "pub fn day_timeline_assemble(");
has("返回体 DayTimeline", rsInsights, "pub struct DayTimeline");
has("行结构 DayTimelineHour", rsInsights, "pub struct DayTimelineHour");
has("前台四类秒逐列输出", rsInsights, "pub slack_secs: i64");
has("查询时归类 category_of", rsInsights, "app_usage::category_of");
has("键鼠口径同周账单 EVENTS_EXPR", rsInsights, "SELECT hour, {EVENTS_EXPR} FROM act_hourly WHERE date = ?1 GROUP BY hour");
has("固定 24 行（0..24）", rsInsights, "(0..24)");
has("工作日口径复用 weekbill::is_workday_of", rsInsights, "is_workday_of(date, hol, &off)");

console.log("== 命令（cmds_bill.rs + 注册链） ==");
has("命令 get_day_timeline", rsBill, "pub(crate) fn get_day_timeline(");
has("offset 钳 0..=1200（防 IPC 入参触发 chrono 日期运算 panic，period_bounds 同口径）", rsBill, "let off = offset.clamp(0, 1200);");
has("锁内取快照、锁外查询", rsBill, "sync::lock(&state.config, \"state.config\").clone()");
has("注册进 generate_handler", rsMain, "get_day_timeline,");
has("capabilities 自动补齐 allow-get-day-timeline", caps, "allow-get-day-timeline");

console.log("== 前端结构 ==");
has("洞察圆点新增时间线", html, '<button class="pg-dot" data-btab="timeline" title="时间线">');
has("pane 容器 billTabTimeline", html, 'id="billTabTimeline"');
has("日导航按钮", html, 'id="tlPrevDay"');
ok("tab 三表接入（pane/key/name）", /timeline: "billTabTimeline"/.test(appSrc) &&
  /"body", "timeline"/.test(appSrc) && /timeline: "时间线"/.test(appSrc));
ok("懒加载分发接入 timeline", /if \(curBillTab === "timeline"\) return loadDayTimeline\(\);/.test(appSrc));
has("invoke 命令名", appSrc, 'invoke("get_day_timeline", { offset })');

console.log("== 前端口径 ==");
ok("未来封顶：› 到今天即禁用/不翻", /if \(next < 0\) return;/.test(appSrc));
ok("凌晨小时仅在有记录时出现（6–23 恒显）", /h\.hour >= 6 \|\| h\.front_secs > 0 \|\| h\.events > 0 \|\| h\.audio_secs > 0/.test(appSrc));
ok("空行全 0 不臆造离开（空态走 tlEmpty）", /这一天没有任何监控记录/.test(html) && /tlEmpty"\)\.classList\.toggle\("hidden", visible\.length > 0\)/.test(appSrc));
ok("构成条按 3600 秒绝对占比（跨小时可比）", /tlPct\(secs, 3600\)/.test(appSrc));
ok("四色构成段齐全", /tl-seg-work/.test(appSrc) && /tl-seg-slack/.test(appSrc) &&
  /tl-seg-comm/.test(appSrc) && /tl-seg-other/.test(appSrc));
ok("行注释含键鼠次数与 top_app", /键鼠 " \+ fmtWan\(h\.events\)/.test(appSrc) && /h\.top_app \? escapeHtml\(h\.top_app\)/.test(appSrc));

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
