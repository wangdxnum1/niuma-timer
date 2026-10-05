// 守护·暂停徽章的回归测试：从前端源码（feSource 聚合）抽出真实 renderBadge，用桩 DOM 跑全部分支。
// 定时休息已砍（通知化改造）：徽章只剩手动暂停一种 off 态，无 rest_secs 分支。
// 用法（任意目录）：node scripts/test_guard.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const appSrc = require("./lib/fe_sources").feSource();
const htmlSrc = fs.readFileSync(path.join(ROOT, "frontend", "index.html"), "utf8");
const capSrc = fs.readFileSync(
  path.join(ROOT, "src-tauri", "capabilities", "default.json"),
  "utf8"
);
// main.rs 模块化拆分后，「死命令不得在任何文件复活」改扫聚合源
const { rsSource } = require("./lib/rs_sources");
const rsSrc = rsSource();

let pass = 0;
let fail = 0;
function eq(label, actual, expect) {
  if (actual === expect) {
    pass++;
    console.log("  PASS " + label + "  ->  " + actual);
  } else {
    fail++;
    console.log("  FAIL " + label + "\n       期望: " + expect + "\n       实际: " + actual);
  }
}
function ok(label, cond) {
  eq(label, !!cond, true);
}

function pick(re, name) {
  const m = appSrc.match(re);
  if (!m) throw new Error("提取失败: " + name);
  return m[0];
}

// 抽取真实函数（renderBadge 依赖 fmtShortH 与 updateLeaveBtn，一并抽出）
const code = [
  pick(/function fmtShortH\(h\) \{[\s\S]*?\n\}/, "fmtShortH"),
  pick(/function renderBadge\(s\) \{[\s\S]*?\n\}/, "renderBadge"),
  pick(/function updateLeaveBtn\(s\) \{[\s\S]*?\n\}/, "updateLeaveBtn"),
].join("\n");

// 桩 DOM：按 id 记录最后一次写入的文案与 class
const els = {};
function makeEl() {
  const classes = new Set();
  return {
    text: "",
    cls: "",
    set textContent(v) {
      this.text = v;
    },
    get textContent() {
      return this.text;
    },
    set className(v) {
      this.cls = v;
      classes.clear();
      v.split(/\s+/).filter(Boolean).forEach((c) => classes.add(c));
    },
    get className() {
      return this.cls;
    },
    classList: {
      add: (c) => classes.add(c),
      remove: (c) => classes.delete(c),
      contains: (c) => classes.has(c),
      toggle: (c, f) => {
        const on = f === undefined ? !classes.has(c) : !!f;
        if (on) classes.add(c);
        else classes.delete(c);
        return on;
      },
    },
    has: (c) => classes.has(c),
  };
}
global.$ = (id) => (els[id] = els[id] || makeEl());

const api = new Function(
  "return (function(){\n" + code + "\n; return { renderBadge };\n})();"
)();

const badge = () => global.$("statusBadge");
const work = (o) =>
  Object.assign({ is_workday: true, off_work: false, worked_h: 4, to_off_h: 3 }, o);

console.log("== 手动暂停分支 ==");
api.renderBadge(work({ paused: true }));
eq("手动暂停", badge().text, "已暂停 · 钱先冻结");
eq("暂停 className", badge().cls, "badge off");
ok("renderBadge 无 rest_secs 残留", !/rest_secs/.test(appSrc));

console.log("== 休假标记分支 ==");
const leaveBtn = () => global.$("leaveBtn");
api.renderBadge(work({ day_off_kind: "年假", is_workday: false }));
eq("休假徽章", badge().text, "休假中 · 年假");
eq("休假 className", badge().cls, "badge off");
eq("已标记 → 取消休假", leaveBtn().text, "取消休假");
ok("已标记按钮可见", !leaveBtn().has("hidden"));
api.renderBadge(work({}));
eq("未标记 → 标记休假", leaveBtn().text, "标记休假");
ok("未标记按钮可见", !leaveBtn().has("hidden"));
api.renderBadge(work({ is_workday: false }));
ok("周末隐藏按钮", leaveBtn().has("hidden"));
api.renderBadge(work({ is_workday: false, day_off_kind: "事假", paused: true }));
ok("暂停优先隐藏按钮", leaveBtn().has("hidden"));

console.log("== 常规状态不受影响 ==");
api.renderBadge(work({}));
eq("搬砖中", badge().text, "● 搬砖中 · 距下班 3.0h");
eq("搬砖中 className", badge().cls, "badge");
api.renderBadge(work({ off_work: true, to_off_h: 0 }));
eq("已下班", badge().text, "已下班 · 辛苦了");
api.renderBadge(work({ is_workday: false }));
eq("今天休息", badge().text, "今天休息");
api.renderBadge(work({ to_off_h: 0.25 }));
eq("临近下班文案", badge().text, "● 搬砖中 · 距下班 15 分钟");

console.log("== 配置接线（守护设置卡）==");
ok(
  "load 回填久坐开关（!== false 缺省 true）",
  /remind_sedentary_enabled"\)\.checked = cfg\.remind_sedentary_enabled !== false/.test(appSrc)
);
ok(
  "load 回填阈值（?? 50 缺省）",
  /remind_sedentary_minutes"\)\.value = cfg\.remind_sedentary_minutes \?\? 50/.test(appSrc)
);
ok(
  "load 回填下班开关",
  /remind_offwork_enabled"\)\.checked = cfg\.remind_offwork_enabled !== false/.test(appSrc)
);
ok(
  "load 回填快捷键开关",
  /shortcuts_enabled"\)\.checked = cfg\.shortcuts_enabled !== false/.test(appSrc)
);
ok("readCfg 采集阈值并钳制上限 120", /remind_sedentary_minutes: Math\.min\(\s*120,/.test(appSrc));
ok("readCfg 阈值钳制下限 1", /Math\.max\(1, parseInt\(/.test(appSrc));
ok(
  "readCfg 采集久坐开关",
  /remind_sedentary_enabled: \$\("remind_sedentary_enabled"\)\.checked/.test(appSrc)
);
ok(
  "readCfg 采集下班开关",
  /remind_offwork_enabled: \$\("remind_offwork_enabled"\)\.checked/.test(appSrc)
);
ok(
  "readCfg 采集快捷键开关",
  /shortcuts_enabled: \$\("shortcuts_enabled"\)\.checked/.test(appSrc)
);
ok(
  "三个守护开关 change 即存",
  /\$\("remind_sedentary_enabled"\)\.addEventListener\("change", saveNow\)/.test(appSrc) &&
    /\$\("remind_offwork_enabled"\)\.addEventListener\("change", saveNow\)/.test(appSrc) &&
    /\$\("shortcuts_enabled"\)\.addEventListener\("change", saveNow\)/.test(appSrc)
);
ok(
  "阈值失焦保存",
  /\$\("remind_sedentary_minutes"\)\.addEventListener\("blur", saveIfChanged\)/.test(appSrc)
);

console.log("== DOM / 后端契约 ==");
ok("守护设置卡存在", /<h2(?:\s[^>]*)?>守护<\/h2>/.test(htmlSrc));
ok("阈值输入 1–120", /id="remind_sedentary_minutes" type="number" min="1" max="120"/.test(htmlSrc));
ok("capabilities 已无 allow-pause-monitor", !/"allow-pause-monitor"/.test(capSrc));
ok("pause_monitor 死命令已删（全 Rust 源）", !/fn pause_monitor\(/.test(rsSrc));

console.log("\n" + pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
