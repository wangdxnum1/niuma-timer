// 导航改版回归测试（macOS 风分段工具栏：主页|明细 两段 + ⚙ 设置齿轮 + 明细二级分段条）：
// 1) index.html：工具栏 2 段（data-nav：viewMain/detail，主页首位）；⚙ 齿轮按钮
//    （gearBtn，data-nav=viewSettings，必须带 aria-label——纯图标按钮无可见文字）；
//    明细二级分段条（id=segbar）4 项 data-nav 覆盖 4 个明细视图
// 2) app.js：旧导航类名（topnav-item / seg-item / rail）与旧入口 id 零残留——
//    残留 $("..") 会拿到 null，addEventListener 直接 TypeError 白屏
// 3) app.js：绑定 .seg-nav 与 gearBtn；明细聚合映射 '? "detail" : id'；
//    分段条仅在明细视图显示；记忆 lastDetailView
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

// ---------------------------------------------------------------- 1. 工具栏与分段结构
const viewIds = [...html.matchAll(/<div class="app[^"]*" id="(view\w+)"/g)].map((m) => m[1]);
// 工具栏分段 = segbar 之外的 seg-nav（避免把二级分段算进主导航）
const DETAIL_VIEWS = ["viewOt", "viewAct", "viewApp", "viewAudio"];
const allNavs = [...html.matchAll(/class="seg-nav[^"]*"\s+data-nav="(\w+)"/g)].map((m) => m[1]);
const toolbarSegs = allNavs.filter((nav) => !DETAIL_VIEWS.includes(nav));
const subSegs = allNavs.filter((nav) => DETAIL_VIEWS.includes(nav));

eq("视图数量（.app）", viewIds.length, 6);
eq("工具栏分段 = 主页/明细 两段", JSON.stringify(toolbarSegs), JSON.stringify(["viewMain", "detail"]));
eq("主页是第一个分段", toolbarSegs[0], "viewMain");
eq("二级分段覆盖 4 个明细视图", JSON.stringify(subSegs), JSON.stringify(DETAIL_VIEWS));
eq(
  "app.js DETAIL_VIEWS 与分段条一致",
  appSrc.includes('const DETAIL_VIEWS = ["viewOt", "viewAct", "viewApp", "viewAudio"];'),
  true
);
// 明细分段必须在 app.js 里映射到「明细」聚合段
eq("明细视图映射到 detail 聚合段", appSrc.includes('? "detail" : id'), true);

// ⚙ 设置齿轮：纯图标按钮，可访问名称必备
eq("html 有设置齿轮 gearBtn", html.includes('id="gearBtn"'), true);
eq("齿轮指向设置视图", /id="gearBtn"[^>]*data-nav="viewSettings"/.test(html), true);
eq("齿轮带 aria-label（图标按钮无可见文字）", /id="gearBtn"[^>]*aria-label="设置"/.test(html), true);

// ---------------------------------------------------------------- 2. 旧导航形态零残留
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
// gearBtn 是新入口，全文件只允许出现一次（id 重复会使 $() 取错元素）
eq("gearBtn 全文件唯一", (html.match(/id="gearBtn"/g) || []).length, 1);
// 三代旧导航类名一律不许再出现：顶栏 tab、独立分段项、左侧 rail
// 注意 mon-seg-item（监控小分段）是合法现役类名，只能做精确类匹配，不能裸查子串
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
}

// ---------------------------------------------------------------- 3. 绑定与高亮同步
eq("app.js 绑定 .seg-nav 点击", appSrc.includes('querySelectorAll(".seg-nav")'), true);
eq("app.js 绑定 gearBtn 点击", appSrc.includes('$("gearBtn").addEventListener'), true);
eq("app.js 工具栏分段依据 data-nav 同步高亮", appSrc.includes("b.dataset.nav === navKey"), true);
eq("app.js 分段条仅在明细视图显示", appSrc.includes('classList.toggle("hidden", !isDetail)'), true);
eq("app.js 记忆上次明细视图（明细段回落目标）", appSrc.includes("lastDetailView"), true);

// ---------------------------------------------------------------- 4. app.js 引用的元素必须存在（全量兜底）
const refIds = [...appSrc.matchAll(/\$\("([A-Za-z_]\w*)"\)/g)].map((m) => m[1]);
const missing = [...new Set(refIds)].filter((id) => !html.includes('id="' + id + '"'));
eq("app.js 引用的元素全部存在于 index.html", JSON.stringify(missing), "[]");

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
