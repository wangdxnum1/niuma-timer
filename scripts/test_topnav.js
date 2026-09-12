// 导航改版回归测试（顶部三 tab + 明细分段条，取代原「‹ 返回」按钮的网页式导航）：
// 1) index.html：顶导 3 个 tab（data-nav），顺序 = 主页首位 / 设置末位（原生惯例）；
//    分段条 4 项（data-seg）与 4 个明细视图一一对应
// 2) app.js：不再引用任何已删除元素（rail 类名、返回按钮、设置入口）。残留 $("..")
//    会取到 null，addEventListener 直接 TypeError 白屏——这是删 DOM 后最容易翻车的方式
// 3) app.js：顶导绑定、分段绑定、tab 高亮映射、分段条显隐逻辑存在
// 4) app.js 引用的所有 $("id") 在 index.html 中都存在（全量兜底，防删漏）
// 运行：node scripts/test_topnav.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const appSrc = fs.readFileSync(path.join(ROOT, "frontend", "app.js"), "utf8");
const html = fs.readFileSync(path.join(ROOT, "frontend", "index.html"), "utf8");

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

// ---------------------------------------------------------------- 1. 视图与导航项结构
const viewIds = [...html.matchAll(/<div class="app[^"]*" id="(view\w+)"/g)].map((m) => m[1]);
const navKeys = [...html.matchAll(/class="topnav-item[^"]*"\s+data-nav="(\w+)"/g)].map(
  (m) => m[1]
);
const segViews = [...html.matchAll(/class="seg-item[^"]*"\s+data-seg="(view\w+)"/g)].map(
  (m) => m[1]
);
const DETAIL_VIEWS = ["viewOt", "viewAct", "viewApp", "viewAudio"];

eq("视图数量（.app）", viewIds.length, 6);
eq("顶导 tab 数量（topnav-item）", navKeys.length, 3);
eq("分段项数量（seg-item）", segViews.length, 4);
eq("顶导顺序 = 主页/明细/设置", JSON.stringify(navKeys), JSON.stringify(["viewMain", "detail", "viewSettings"]));
// 分段条必须覆盖全部 4 个明细视图，且与 app.js 的 DETAIL_VIEWS 契约一致
eq("分段覆盖 4 个明细视图", JSON.stringify(segViews), JSON.stringify(DETAIL_VIEWS));
eq(
  "app.js DETAIL_VIEWS 与分段条一致",
  appSrc.includes('const DETAIL_VIEWS = ["viewOt", "viewAct", "viewApp", "viewAudio"];'),
  true
);
// 明细分段必须在 app.js 里映射到「明细」聚合 tab
eq("明细视图映射到 detail 聚合 tab", appSrc.includes('? "detail" : id'), true);

// ---------------------------------------------------------------- 2. 已删除元素不得再被引用
// 这些元素/类名已从 index.html 删除；app.js 残留 $("xxx") 会拿到 null，
// 后面紧跟的 addEventListener 在启动时即抛 TypeError → 白屏
const removedIds = [
  "settingsBackBtn",
  "otBackBtn",
  "actBackBtn",
  "appuBackBtn",
  "audioBackBtn",
  "settingsBtn",
];
for (const id of removedIds) {
  eq("html 无残留 " + id, html.includes('id="' + id + '"'), false);
  eq("app.js 无残留 " + id, appSrc.includes('"' + id + '"'), false);
}
// 侧栏方案整体下线：rail 类名不允许在任何前端文件出现
for (const [name, src] of [["html", html], ["app.js", appSrc]]) {
  eq(name + " 无残留 rail 类名", /class="[^"]*\brail[-\s"]|querySelectorAll\("\.rail/.test(src), false);
}

// ---------------------------------------------------------------- 3. 顶导 / 分段绑定与高亮同步
eq(
  "app.js 绑定 .topnav-item 点击",
  appSrc.includes('querySelectorAll(".topnav-item")'),
  true
);
eq(
  "app.js 绑定 .seg-item 点击",
  appSrc.includes('querySelectorAll(".seg-item")'),
  true
);
eq(
  "app.js 顶导依据 data-nav 同步高亮",
  appSrc.includes('b.dataset.nav === navKey'),
  true
);
eq(
  "app.js 分段依据 data-seg 同步高亮",
  appSrc.includes('b.dataset.seg === id'),
  true
);
eq(
  "app.js 分段条仅在明细视图显示",
  appSrc.includes('classList.toggle("hidden", !isDetail)'),
  true
);
eq(
  "app.js 记忆上次明细视图（明细 tab 回落目标）",
  appSrc.includes("lastDetailView"),
  true
);

// ---------------------------------------------------------------- 4. app.js 引用的元素必须存在（全量兜底）
const refIds = [...appSrc.matchAll(/\$\("([A-Za-z_]\w*)"\)/g)].map((m) => m[1]);
const missing = [...new Set(refIds)].filter(
  (id) => !html.includes('id="' + id + '"')
);
eq("app.js 引用的元素全部存在于 index.html", JSON.stringify(missing), "[]");

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
