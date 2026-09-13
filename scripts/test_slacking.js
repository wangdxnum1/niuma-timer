// 摸鱼统计回归测试（应用分类：工作/摸鱼/沟通/其他）：
// 1) index.html：应用明细页有「今日构成」卡（catSummary 容器 + 说明文案）
// 2) app.js：分类循环表 CAT_CYCLE（四类固定序）、catKey 配色映射、
//    renderCategories 构成条渲染、cycleAppCategory 走 load_config→改 map→
//    save_config 的覆盖写路径、主页监控卡摸鱼速览行（mon-slack）
// 3) renderAppRows 支持 opts.chips 分类标签（仅明细页开启）
// 运行：node scripts/test_slacking.js
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
function has(label, src, needle) {
  eq(label, src.includes(needle), true);
}

// ---------------------------------------------------------------- 1. index.html 结构
eq("明细页有分类汇总容器 catSummary", html.includes('id="catSummary"'), true);
has("构成卡说明文案（可点标签改分类）", html, "点下方应用行的分类标签可改");

// ---------------------------------------------------------------- 2. app.js 分类逻辑
has("分类循环表 CAT_CYCLE", appSrc, 'const CAT_CYCLE = ["工作", "摸鱼", "沟通", "其他"];');
has("catKey 配色映射", appSrc, "function catKey(label)");
has("构成条渲染 renderCategories", appSrc, "function renderCategories(s)");
has("明细页接入构成渲染", appSrc, "renderCategories(s);");
has("改分类走覆盖写路径", appSrc, "async function cycleAppCategory(app, current)");
has("改分类读取现有 map 再改", appSrc, "Object.assign({}, cfg.app_categories || {})");
has("改分类后保存", appSrc, 'invoke("save_config", { cfg: { app_categories: map } })');
has("改分类后刷新视图", appSrc, "loadAppUsage();");
// 主页监控卡摸鱼速览行
has("主页摸鱼速览行", appSrc, 'class="mon-slack"');
has("速览行引用工作构成", appSrc, 'c.key === "work"');
has("速览行引用摸鱼构成", appSrc, 'c.key === "slack"');
// 应用行分类标签（仅明细页 chips 模式）
has("renderAppRows 支持 chips 选项", appSrc, "opts.chips");
eq("chip 标注可点切换", appSrc.includes("点击切换分类"), true);
has("明细页开启 chips", appSrc, '{ chips: true }');

// ---------------------------------------------------------------- 3. 兜底：$() 引用存在
const refIds = [...appSrc.matchAll(/\$\("([A-Za-z_]\w*)"\)/g)].map((m) => m[1]);
const missing = [...new Set(refIds)].filter((id) => !html.includes('id="' + id + '"'));
eq("app.js 引用的元素全部存在于 index.html", JSON.stringify(missing), "[]");

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
