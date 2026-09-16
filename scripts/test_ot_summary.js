// 加班明细「合计」展示升级回归测试：
// 把原先不起眼的一行灰字（.mn-summary）升级为金色统计卡（.ot-summary），
// 照搬主页「本月加班战果」金卡视觉语言——大号金色金额 + 时长/天数/日均 三格。
// 锁住：① 视觉升级确实落地（HTML 换类、CSS 金卡样式存在）
//       ② 三个真实字段（total_all / total_hours / days）仍流入渲染，不退化成写死文案
//       ③ 空态文案与 empty 类切换仍在
// 风格参照 scripts/test_week_bill.js（源码扫描 has 断言，不依赖真实 DOM）。
// 运行：node scripts/test_ot_summary.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const appSrc = fs.readFileSync(path.join(ROOT, "frontend", "app.js"), "utf8");
const html = fs.readFileSync(path.join(ROOT, "frontend", "index.html"), "utf8");
const css = fs.readFileSync(path.join(ROOT, "frontend", "styles.css"), "utf8");

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

// ---------------------------------------------------------------- 1. HTML 容器升级
has("HTML: otMonthSummary 升级为金卡类 ot-summary",
  html, 'id="otMonthSummary" class="ot-summary"');
eq("HTML: 不再用旧类 mn-summary 包裹",
  html.includes('id="otMonthSummary" class="mn-summary"'), false);

// ---------------------------------------------------------------- 2. app.js 渲染逻辑
has("app: 渲染金额 total_all", appSrc, "ot.total_all.toFixed(0)");
has("app: 渲染时长 total_hours", appSrc, "ot.total_hours.toFixed(1)");
has("app: 渲染天数 days", appSrc, "ot.days");
has("app: 金卡结构 os-amount", appSrc, "os-amount");
has("app: 金卡结构 os-meta", appSrc, "os-meta");
has("app: 空态切 classList.add('empty')", appSrc, 'sum.classList.add("empty")');
has("app: 非空清 classList.remove('empty')", appSrc, 'sum.classList.remove("empty")');
has("app: 空态文案 该月暂无加班记录", appSrc, "该月暂无加班记录");

// ---------------------------------------------------------------- 3. CSS 金卡样式
has("CSS: .ot-summary 卡", css, ".ot-summary {");
has("CSS: .ot-summary.empty 空态", css, ".ot-summary.empty {");
has("CSS: .os-amount 金色金额", css, ".os-amount {");
has("CSS: .os-meta .cell 统计格", css, ".os-meta .cell {");
has("CSS: 金色金额色值 #ffd650", css, "#ffd650");

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
