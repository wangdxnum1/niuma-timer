// 分类智能建议（v1.7.0）契约测试：
//  1) Rust：关键词规则表 + 守卫（已分类不建议）+ summary 注入条件（其他类+600 秒）
//  2) 前端：建议 chip 渲染条件 + acceptSuggestion 落库路径 + 事件委托
//  3) 不自动生效：建议只在用户点击后写入
//
// 运行：node scripts/test_suggest_category.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const read = (p) => fs.readFileSync(path.join(ROOT, p), "utf-8").replace(/\r\n/g, "\n");
const rsApp = read("src-tauri/src/app_usage.rs");
const html = read("frontend/index.html");
const monitorSrc = read("frontend/js/monitor.js");
const bootSrc = read("frontend/js/boot.js");
const css = read("frontend/styles.css");

let pass = 0, fail = 0;
const eq = (name, a, b) => {
  if (a === b) { pass++; } else { fail++; console.log("FAIL " + name + "  期望 " + JSON.stringify(b) + " 实际 " + JSON.stringify(a)); }
};
const ok = (name, cond) => eq(name, !!cond, true);
const has = (name, hay, needle) => eq(name, typeof hay === "string" && hay.includes(needle), true);

console.log("== Rust 规则表 ==");
has("suggest_category 纯规则函数", rsApp, "pub(crate) fn suggest_category(display: &str, cfg: &Config) -> Option<String>");
ok("守卫：非其他类直接不建议", /if category_of\(display, cfg\) != CAT_OTHER \{\s*\n\s*return None;/.test(rsApp));
ok("关键词大小写不敏感", rsApp.includes("let low = display.to_lowercase();"));
ok("规则覆盖三类（工作/沟通/摸鱼）", rsApp.includes('("code", CAT_WORK)') &&
  rsApp.includes('("wechat", CAT_COMM)') && rsApp.includes('("bilibili", CAT_SLACK)'));
ok("summary 注入条件：其他类且当日 ≥600 秒", /category == CAT_OTHER && seconds >= 600 \{\s*\n\s*suggest_category/.test(rsApp));
ok("单测覆盖（命中/默认表排除/用户覆盖排除/无命中）",
  rsApp.includes("fn suggest_category_rules"));
ok("AppUsageItem 带 suggestion 字段", rsApp.includes("pub suggestion: Option<String>"));

console.log("== 前端渲染与交互 ==");
ok("建议 chip 渲染条件：其他类且有建议", /chips && a\.category === "其他" && a\.suggestion/.test(monitorSrc));
has("chip 类名与提示", monitorSrc, 'class="cat-chip cat-suggest"');
ok("chip 文案带建议分类", monitorSrc.includes("建议："));
ok("acceptSuggestion 直写分类（load_config → app_categories → save_config → 重算）",
  /async function acceptSuggestion\(app, cat\) \{[\s\S]{0,300}save_config", \{ cfg: \{ app_categories: map \} \}[\s\S]{0,120}loadAppUsage\(\);/.test(monitorSrc));
ok("事件委托先建议后循环分类", /const sug = e\.target\.closest\("\.cat-suggest"\);/.test(bootSrc) &&
  bootSrc.includes("acceptSuggestion(sug.dataset.sapp, sug.dataset.scat)"));
ok("建议 chip 虚线金描边样式", css.includes(".cat-suggest"));
ok("不自动生效：渲染层不调用 save_config 写建议",
  !/renderAppRows[\s\S]{0,2000}save_config/.test(monitorSrc));

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
