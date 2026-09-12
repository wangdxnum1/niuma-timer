// 导航回归测试（Windows 11 设置风格侧边栏：主页/明细/设置 三项纵排，64px 图标+标签）：
// 1) index.html：侧栏 .rail 三项（data-nav：viewMain/detail/viewSettings），
//    主页首位、设置沉底（原生惯例）；每项含图标 + 可见文字标签（可访问名齐备）；
//    翻页器 detailPager 默认隐藏，pgPrev/pgNext/pgName + 4 个 pg-dot 按序覆盖明细视图
// 2) app.js：历代旧导航零残留——顶栏 tab、独立分段项、二级分段条、齿轮按钮；
//    旧返回按钮等 id 同样零残留（残留 $("..") 启动即 TypeError 白屏）
// 3) app.js：侧栏绑定、明细聚合映射 '? "detail" : id'、翻页器仅在明细视图显示、
//    记忆 lastDetailView、滚轮翻页（wheel 监听 + 顶/底边界 + 冷却防惯性连翻）
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

// ---------------------------------------------------------------- 1. 侧栏与翻页器结构
const viewIds = [...html.matchAll(/<div class="app[^"]*" id="(view\w+)"/g)].map((m) => m[1]);
const DETAIL_VIEWS = ["viewOt", "viewAct", "viewApp", "viewAudio"];
const railNavs = [...html.matchAll(/class="rail-item[^"]*"\s+data-nav="(\w+)"/g)].map((m) => m[1]);
const dotNavs = [...html.matchAll(/class="pg-dot[^"]*"\s+data-nav="(view\w+)"/g)].map((m) => m[1]);

eq("视图数量（.app）", viewIds.length, 6);
eq("侧栏三项 = 主页/明细/设置", JSON.stringify(railNavs), JSON.stringify(["viewMain", "detail", "viewSettings"]));
eq("主页是侧栏第一项", railNavs[0], "viewMain");
eq("设置沉底（原生惯例）", railNavs[railNavs.length - 1], "viewSettings");
eq("翻页器圆点按序覆盖 4 个明细视图", JSON.stringify(dotNavs), JSON.stringify(DETAIL_VIEWS));
eq(
  "app.js DETAIL_VIEWS 与圆点顺序一致",
  appSrc.includes('const DETAIL_VIEWS = ["viewOt", "viewAct", "viewApp", "viewAudio"];'),
  true
);
eq("明细视图映射到 detail 聚合项", appSrc.includes('? "detail" : id'), true);

// 侧栏项可见标签（图标+文字双通道，无纯图标入口）
for (const t of [">主页<", ">明细<", ">设置<"]) {
  eq("侧栏有可见标签 " + t, html.includes('rail-txt">' + t.slice(1)), true);
}

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
  "gearBtn",
  "segbar",
  "subbar",
];
for (const id of removedIds) {
  eq("html 无残留 " + id, html.includes('id="' + id + '"'), false);
  if (!["segbar", "subbar"].includes(id)) {
    eq("app.js 无残留 " + id, appSrc.includes('"' + id + '"'), false);
  }
}
// 历代旧导航类名/结构一律不许再出现：顶栏 tab、独立分段项、二级分段条、齿轮
for (const [name, src] of [
  ["html", html],
  ["app.js", appSrc],
]) {
  eq(name + " 无残留 topnav", src.includes("topnav"), false);
  eq(name + " 无残留 toolbar", src.includes("toolbar"), false);
  eq(name + " 无残留 seg-nav", src.includes("seg-nav"), false);
  eq(
    name + " 无残留 seg-item 精确类",
    /class="seg-item[ "]/.test(src) || src.includes('".seg-item"'),
    false
  );
  eq(name + " 无残留二级分段条 segbar/subbar", /id="(?:segbar|subbar)"|getElementById\("(?:segbar|subbar)"\)/.test(src), false);
}

// ---------------------------------------------------------------- 3. 侧栏与翻页器绑定
eq("app.js 绑定 .rail-item 点击", appSrc.includes('querySelectorAll(".rail-item")'), true);
eq("app.js 侧栏按 navKey 同步高亮", appSrc.includes("b.dataset.nav === navKey"), true);
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
