// 专注段统计（v1.7.0）契约测试：
//  1) Rust：状态机（诚实结算口径）+ 聚合 + 命令注册链 + capabilities
//  2) 前端：洞察第 6 tab 接入 + 守护卡三处接线
//  3) 口径：无输入 5 分钟断段、暂停/远程断段、段结束才落库、空态如实
//
// 运行：node scripts/test_focus.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const read = (p) => fs.readFileSync(path.join(ROOT, p), "utf-8").replace(/\r\n/g, "\n");
const rsFocus = read("src-tauri/src/focus.rs");
const rsActivity = read("src-tauri/src/activity.rs");
const rsApp = read("src-tauri/src/app_usage.rs");
const rsDb = read("src-tauri/src/db.rs");
const rsMaintain = read("src-tauri/src/maintain.rs");
const rsSched = read("src-tauri/src/scheduler.rs");
const rsBill = read("src-tauri/src/cmds_bill.rs");
const rsMain = read("src-tauri/src/main.rs");
const caps = read("src-tauri/capabilities/default.json");
const html = read("frontend/index.html");
const { feSource } = require("./lib/fe_sources");
const appSrc = feSource().replace(/\r\n/g, "\n");

let pass = 0, fail = 0;
const eq = (name, a, b) => {
  if (a === b) { pass++; } else { fail++; console.log("FAIL " + name + "  期望 " + JSON.stringify(b) + " 实际 " + JSON.stringify(a)); }
};
const ok = (name, cond) => eq(name, !!cond, true);
const has = (name, hay, needle) => eq(name, typeof hay === "string" && hay.includes(needle), true);

console.log("== Rust 状态机（focus.rs） ==");
has("纯状态机 tick_run（显式输入，不碰全局）", rsFocus, "pub(crate) fn tick_run(");
has("系统级 idle 口径", rsFocus, "crate::win::idle_ms()");
has("无输入 5 分钟断段", rsFocus, "const IDLE_GAP_MS: u64 = 5 * 60 * 1000;");
has("远程会话断段（与加班排除同判定）", rsFocus, "remote::is_remote_active(remote::REMOTE_WINDOW)");
has("手动暂停断段", rsFocus, "pause::is_paused()");
ok("诚实结算：时长按最后确认活跃计（断段拍不延长）",
  rsFocus.includes("时长按「最后一次确认活跃」计") && rsFocus.includes("r.last_active_ms = input.now_ms;"));
ok("不足阈值丢弃（fail-closed）",
  rsFocus.includes("if elapsed_ms >= threshold_min * 60_000 {"));
has("段结束才落库一条", rsFocus, "INSERT INTO focus_sessions");
has("跨午夜归属开始日", rsFocus, "次日 {eh}");
has("阈值钳制 10–120", rsFocus, "cfg.focus_min_minutes.clamp(10, 120)");

console.log("== 信号与接线 ==");
ok("信号来自系统 idle（app_usage 前台访问器）", rsApp.includes("pub(crate) fn current_display()"));
has("scheduler 每拍喂信号", rsSched, 'run("focus_tick", move || crate::focus::tick(&cfg));');
has("main.rs 注册 mod focus", rsMain, "mod focus;");
has("capabilities 自动补齐 allow-get-focus-summary", caps, "allow-get-focus-summary");
has("focus_sessions 纳入保留期清理", rsMaintain, '("focus_sessions", "date"),');
has("建表语句入 TABLE_DDL", rsDb, "CREATE_FOCUS_SESSIONS,");
ok("单测覆盖关键分支", rsFocus.includes("fn qualifies_after_threshold_and_closes") &&
  rsFocus.includes("fn below_threshold_is_discarded") &&
  rsFocus.includes("fn slack_foreground_breaks_session") &&
  rsFocus.includes("fn idle_gap_breaks_and_honest_minutes") &&
  rsFocus.includes("fn summary_groups_and_picks_best_day"));

console.log("== 命令与洞察 tab ==");
has("命令 get_focus_summary", rsBill, "pub(crate) fn get_focus_summary(");
has("注册进 generate_handler", rsMain, "get_focus_summary,");
has("洞察圆点", html, '<button class="pg-dot" data-btab="focus" title="专注">');
has("pane 容器", html, 'id="billTabFocus"');
ok("tab 三表接入", /focus: "billTabFocus"/.test(appSrc) &&
  /"timeline", "focus"/.test(appSrc) && /focus: "专注"/.test(appSrc));
ok("懒加载分发接入", /if \(curBillTab === "focus"\) return loadFocusSummary\(\);/.test(appSrc));
has("invoke 命令名", appSrc, 'invoke("get_focus_summary", { span, offset })');
ok("空态如实（自 v1.7.0 起积累）", html.includes("专注统计自 v1.7.0 起积累"));

console.log("== 守护卡接线 ==");
ok("守护卡两控件", html.includes('id="focus_enabled"') && html.includes('id="focus_min_minutes"'));
ok("load 回填（开关缺省 true、阈值缺省 25）",
  appSrc.includes('cfg.focus_enabled !== false') && appSrc.includes("cfg.focus_min_minutes ?? 25"));
ok("readCfg 采集并钳制 10–120",
  /focus_enabled: \$\("focus_enabled"\)\.checked,/.test(appSrc) &&
  appSrc.includes("focus_min_minutes: Math.min(") && appSrc.includes("Math.max(10, parseInt"));
ok("绑定：开关即存 + 阈值失焦存",
  appSrc.includes('$("focus_enabled").addEventListener("change", saveNow);') &&
  appSrc.includes('$("focus_min_minutes").addEventListener("blur", saveIfChanged);'));

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
