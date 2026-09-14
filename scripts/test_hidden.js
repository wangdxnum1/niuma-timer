// 隐藏契约检查：凡是在 index.html 里初始带 `hidden` 类的元素，
// styles.css 必须存在一条能真正把它藏起来的规则——要么是通用 `.hidden`，
// 要么是针对该元素形态类的 `.base.hidden`。
//
// 背景（2026-09-14 bug）：周账单三面板 #billEmpty / #billReceipt / #billDash
// 都在 HTML 里写了 `class="... hidden"`，但 styles.css 里从没给它们配
// `.xxx.hidden { display:none }`，也没有通用 `.hidden`。而 .bill-dash / .bill-empty
// 自带 `display:flex`，于是 hidden 类完全失效 —— 结果账单页三块同时渲染：
// 顶部误显空态文案、底部露出未填充的仪表盘骨架（¥0.00 + 四个「—」+ 空卡片）。
// 这类"class 写了但 CSS 没兜住"的问题肉眼极难发现，必须让它在测试里红。
//
// 运行：node scripts/test_hidden.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const css = fs.readFileSync(path.join(ROOT, "frontend", "styles.css"), "utf8");
const html = fs.readFileSync(path.join(ROOT, "frontend", "index.html"), "utf8");
const appSrc = fs.readFileSync(path.join(ROOT, "frontend", "app.js"), "utf8");

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
const esc = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

// ---- 1. 收集 index.html 里所有「id + class 含 hidden」的元素 ----
const elements = [];
const tagRe = /<[a-z][^>]*>/gi;
let t;
while ((t = tagRe.exec(html)) !== null) {
  const tag = t[0];
  const idm = tag.match(/\bid="([^"]+)"/);
  const clm = tag.match(/\bclass="([^"]*)"/);
  if (!idm || !clm) continue;
  const classes = clm[1].split(/\s+/).filter(Boolean);
  if (!classes.includes("hidden")) continue;
  elements.push({ id: idm[1], classes: classes.filter((c) => c !== "hidden") });
}

console.log("== index.html 初始带 hidden 的元素 ==");
eq("元素数 > 0", elements.length > 0, true);

// ---- 2. 是否存在通用 .hidden { display:none } ----
// 需跨行匹配规则体；前缀用空白/符号界定，避免把 `.subnav.hidden` 这类
// 复合选择器误当成通用规则（区分「独立 .hidden 类选择器」与「.x.hidden」）
const globalRe = /(?:^|[\s},;])\.hidden\s*\{[^}]*display\s*:\s*none/m;
const hasGlobal = globalRe.test(css);
console.log("== 通用 .hidden 工具类 ==");
eq("styles.css 存在通用 .hidden 隐藏规则", hasGlobal, true);

// ---- 3. 每个 hidden 元素都要有可用的隐藏手段 ----
console.log("== 逐个元素的隐藏规则 ==");
const dead = [];
for (const el of elements) {
  if (hasGlobal) break; // 通用规则一劳永逸
  const covered = el.classes.some((c) =>
    new RegExp("\\." + esc(c) + "\\.hidden\\b").test(css)
  );
  if (!covered) dead.push("#" + el.id + "(" + el.classes.join(".") + ")");
}
eq(
  "无隐藏规则的 hidden 元素",
  dead.length ? dead.join(", ") : "无",
  "无"
);

// ---- 4. （反向）app.js 里 classList 切换 hidden 的目标 id 必须存在于 index.html ----
const idsInHtml = new Set();
const allIdRe = /\bid="([^"]+)"/g;
let m;
while ((m = allIdRe.exec(html)) !== null) idsInHtml.add(m[1]);

const toggled = new Set();
const togRe = /\$\("([^"]+)"\)\.classList\.(?:add|remove|toggle)\(\s*"hidden"/g;
while ((m = togRe.exec(appSrc)) !== null) toggled.add(m[1]);
console.log("== app.js hidden 切换目标 ==");
eq("切换目标数 > 0", toggled.size > 0, true);
const ghost = [...toggled].filter((id) => !idsInHtml.has(id));
eq("切换的目标 id 在 index.html 不存在", ghost.length ? ghost.join(", ") : "无", "无");

console.log("");
console.log(
  "  隐藏元素 " + elements.length + " 个，切换目标 " + toggled.size + " 个"
);
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
