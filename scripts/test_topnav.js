// 明细导航回归测试（翻页器：‹ 页名 › + 圆点直达 + 滚轮翻页，取代二级分段条 tab 行）：
// 1) index.html：工具栏 2 段（主页/明细）+ ⚙ 齿轮（aria-label 必备）；
//    翻页器 detailPager 默认隐藏，含 pgPrev/pgNext/pgName 与 4 个 pg-dot
//    （data-nav 按顺序覆盖 4 个明细视图）
// 2) app.js：旧导航形态零残留——subbar/segbar/seg-item/rail 一律不许出现；
//    旧返回按钮与设置入口 id 同样零残留（残留 $("..") 启动即 TypeError 白屏）
// 3) app.js：翻页器绑定（pgPrev/pgNext 循环、pg-dot 直达）、明细聚合映射
//    '? "detail" : id'、翻页器仅在明细视图显示、记忆 lastDetailView、
//    滚轮翻页（wheel 监听 + 顶/底边界判定 + 冷却防惯性连翻）
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

// ---------------------------------------------------------------- 1. 工具栏与翻页器结构
const viewIds = [...html.matchAll(/<div class="app[^"]*" id="(view\w+)"/g)].map((m) => m[1]);
const DETAIL_VIEWS = ["viewOt", "viewAct", "viewApp", "viewAudio"];
const toolbarSegs = [...html.matchAll(/class="seg-nav[^"]*"\s+data-nav="(\w+)"/g)].map(
  (m) => m[1]
);
const dotNavs = [...html.matchAll(/class="pg-dot[^"]*"\s+data-nav="(view\w+)"/g)].map(
  (m) => m[1]
);

eq("视图数量（.app）", viewIds.length, 6);
eq("工具栏分段 = 主页/明细 两段", JSON.stringify(toolbarSegs), JSON.stringify(["viewMain", "detail"]));
eq("主页是第一个分段", toolbarSegs[0], "viewMain");
eq("翻页器圆点按序覆盖 4 个明细视图", JSON.stringify(dotNavs), JSON.stringify(DETAIL_VIEWS));
eq(
  "app.js DETAIL_VIEWS 与圆点顺序一致",
  appSrc.includes('const DETAIL_VIEWS = ["viewOt", "viewAct", "viewApp", "viewAudio"];'),
  true
);
// 明细视图必须在 app.js 里映射到「明细」聚合段
eq("明细视图映射到 detail 聚合段", appSrc.includes('? "detail" : id'), true);

// ⚙ 设置齿轮：纯图标按钮，可访问名称必备
eq("html 有设置齿轮 gearBtn", html.includes('id="gearBtn"'), true);
eq("齿轮指向设置视图", /id="gearBtn"[^>]*data-nav="viewSettings"/.test(html), true);
eq("齿轮带 aria-label（图标按钮无可见文字）", /id="gearBtn"[^>]*aria-label="设置"/.test(html), true);

// 翻页器：默认隐藏（非明细视图不占位），三件套齐全
eq("翻页器默认隐藏", html.includes('class="subnav hidden" id="detailPager"'), true);
for (const id of ["pgPrev", "pgNext", "pgName"]) {
  eq("html 有翻页器元素 " + id, html.includes('id="' + id + '"'), true);
}
eq("翻页器圆点数量 = 4", dotNavs.length, 4);

// ---------------------------------------------------------------- 2. 旧导航形态零残留
const removedIds = [
  "settingsBackBtn",
  "otBackBtn",
  "actBackBtn",
  "appuBackBtn",
  "audioBackBtn",
  "settingsBtn",
  "segbar",
  "subbar",
];
for (const id of removedIds) {
  eq("html 无残留 " + id, html.includes('id="' + id + '"'), false);
  if (id !== "segbar" && id !== "subbar") {
    eq("app.js 无残留 " + id, appSrc.includes('"' + id + '"'), false);
  }
}
eq("gearBtn 全文件唯一", (html.match(/id="gearBtn"/g) || []).length, 1);
// 历代旧导航类名/结构一律不许再出现：顶栏 tab、独立分段项、左侧 rail、二级分段条
for (const [name, src] of [
  ["html", html],
  ["app.js", appSrc],
]) {
  eq(name + " 无残留 topnav", src.includes("topnav"), false);
  eq(
    name + " 无残留 seg-item 精确类",
    /class="seg-item[ "]/.test(src) || src.includes('".seg-item"'),
    false
  );
  eq(name + " 无残留 rail 类名", /class="[^"]*\brail[-\s"]|querySelectorAll\("\.rail/.test(src), false);
  eq(name + " 无残留二级分段条 segbar/subbar", /id="(?:segbar|subbar)"|getElementById\("(?:segbar|subbar)"\)/.test(src), false);
}

// ---------------------------------------------------------------- 3. 翻页器绑定与滚轮翻页
eq("app.js 绑定 .pg-dot 直达", appSrc.includes('querySelectorAll(".pg-dot")'), true);
eq("app.js 绑定 pgPrev", appSrc.includes('$("pgPrev").addEventListener'), true);
eq("app.js 绑定 pgNext", appSrc.includes('$("pgNext").addEventListener'), true);
eq("app.js 翻页器仅在明细视图显示", appSrc.includes('classList.toggle("hidden", !isDetail)'), true);
eq("app.js 记忆上次明细视图", appSrc.includes("lastDetailView"), true);
// 滚轮翻页：wheel 监听 + 顶/底边界 + 冷却防惯性连翻（断言容忍换行/空白）
eq("app.js 挂载滚轮翻页监听", /addEventListener\(\s*"wheel"/.test(appSrc), true);
const wheelChecks = [
  ["滚轮到底才翻下一页", "atBottom"],
  ["滚轮在顶才翻上一页", "atTop"],
  ["翻页后设冷却防惯性连翻", "FlipUntil = Date.now()"],
];
for (const [label, needle] of wheelChecks) {
  eq(label, appSrc.includes(needle), true);
}

// ---------------------------------------------------------------- 4. app.js 引用的元素必须存在（全量兜底）
const refIds = [...appSrc.matchAll(/\$\("([A-Za-z_]\w*)"\)/g)].map((m) => m[1]);
const missing = [...new Set(refIds)].filter((id) => !html.includes('id="' + id + '"'));
eq("app.js 引用的元素全部存在于 index.html", JSON.stringify(missing), "[]");

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
