// 主页 hero 化回归测试（赚钱进度条 + 监控三合一 + 切换动效）：
// 1) 赚钱进度条：live 卡内 4 个元素齐全；tick() 用 DayStatus 现成字段
//    （hourly_rate × daily_hours）计算、休息日/零时薪隐藏、封顶 100%；
//    样式有金色填充与 width 过渡
// 2) 监控三合一：mon-seg 3 项与 3 个预览面板一一对应；「查看明细」跟随分段；
//    旧的三张卡按钮（actDetailBtn/appuDetailBtn/audioDetailBtn）从 html 与
//    app.js 双侧移除——残留 $("..") 会拿到 null，启动即 TypeError 白屏
// 3) hero 与动效：.live 有 hero 块与放大字号；.app 有 viewIn 动画且尊重
//    prefers-reduced-motion
// 4) app.js 引用的所有 $("id") 在 index.html 中都存在（全量兜底，防删漏）
// 运行：node scripts/test_hero.js
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

// ---------------------------------------------------------------- 1. 赚钱进度条
for (const id of ["liveProgress", "lpFill", "lpTarget", "lpPct"]) {
  eq("html 有进度条元素 " + id, html.includes('id="' + id + '"'), true);
}
eq("进度条默认隐藏（无数据时不占位）", html.includes('class="live-progress hidden"'), true);
has("tick() 引用 is_workday", appSrc, "s.is_workday && s.hourly_rate > 0");
has("全天应赚 = hourly_rate × daily_hours", appSrc, "s.hourly_rate * s.daily_hours");
has("进度封顶 100%", appSrc, "Math.min(100,");
has("填充宽度写入 lpFill", appSrc, '$("lpFill").style.width');
has("隐藏分支存在（休息日/零时薪）", appSrc, 'liveProgress.classList.add("hidden")');
has("金色填充样式", css, ".lp-fill");
has("填充有 width 过渡（金额爬升平滑）", css, "transition: width");
has("说明行样式", css, ".lp-cap");

// ---------------------------------------------------------------- 2. 监控三合一
const monSegs = [...html.matchAll(/class="mon-seg-item[^"]*"\s+data-mon="(\w+)"/g)].map(
  (m) => m[1]
);
eq("监控小分段 3 项", monSegs.length, 3);
eq(
  "分段顺序 = 活动/应用/媒体",
  JSON.stringify(monSegs),
  JSON.stringify(["act", "app", "audio"])
);
for (const id of ["monitorCard", "monAct", "monApp", "monAudio", "monDetailBtn"]) {
  eq("html 有监控卡元素 " + id, html.includes('id="' + id + '"'), true);
}
// 预览面板内的渲染目标 id 必须保留（paint 函数仍在向它们写入）
for (const id of ["act_left", "act_right", "act_keys", "act_hours", "appuHomeList", "audioHomeList"]) {
  eq("渲染目标 id 保留 " + id, html.includes('id="' + id + '"'), true);
}
// 分段 ↔ 面板 / 视图 的映射契约
has("面板映射 MON_PANES", appSrc, 'MON_PANES = { act: "monAct", app: "monApp", audio: "monAudio" }');
has("视图映射 MON_VIEWS", appSrc, 'MON_VIEWS = { act: "viewAct", app: "viewApp", audio: "viewAudio" }');
has("绑定 .mon-seg-item 点击", appSrc, 'querySelectorAll(".mon-seg-item")');
has("查看明细跟随当前分段", appSrc, "MON_VIEWS[curMon]");
// 旧按钮双侧移除
for (const id of ["actDetailBtn", "appuDetailBtn", "audioDetailBtn"]) {
  eq("html 无残留 " + id, html.includes('id="' + id + '"'), false);
  eq("app.js 无残留 " + id, appSrc.includes('"' + id + '"'), false);
}

// ---------------------------------------------------------------- 3. hero 与切换动效
has("hero 块存在", css, ".live {");
eq(
  "hero 大字放大到 46px",
  css.match(/\.live \.big \{[^}]*font-size: (\d+)px/)?.[1] === "46",
  true
);
has("viewIn 关键帧", css, "@keyframes viewIn");
has(".app 挂载 viewIn 动画", css, "animation: viewIn 0.15s ease-out");
has("尊重减弱动效设置", css, "prefers-reduced-motion: reduce");

// ---------------------------------------------------------------- 4. $() 引用兜底
const refIds = [...appSrc.matchAll(/\$\("([A-Za-z_]\w*)"\)/g)].map((m) => m[1]);
const missing = [...new Set(refIds)].filter(
  (id) => !html.includes('id="' + id + '"')
);
eq("app.js 引用的元素全部存在于 index.html", JSON.stringify(missing), "[]");

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
