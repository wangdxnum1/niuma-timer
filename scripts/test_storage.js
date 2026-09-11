// 数据存储「总占用 + 分类细分」的回归测试：
// 从 app.js 抽出真实的 fmtBytes / stgPct / renderStorageInfo，配桩 DOM 跑断言。
// 运行：node scripts/test_storage.js
//
// 关键约束：桩数据**不能手写字段名**。后端结构体是 snake_case 序列化，
// 曾经前端写成 totalBytes 导致总计恒为 0 B、每行占比恒为 0 —— 而测试因为
// 桩数据也跟着写 camelCase 而全绿。所以这里两层防：① 载荷字段与 maintain.rs
// 的 StorageInfo 逐字比对；② 反向扫描 renderStorageInfo 读的字段必须存在。
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const appSrc = fs.readFileSync(path.join(ROOT, "frontend", "app.js"), "utf8");
const css = fs.readFileSync(path.join(ROOT, "frontend", "styles.css"), "utf8");
const rustSrc = fs.readFileSync(
  path.join(ROOT, "src-tauri", "src", "maintain.rs"),
  "utf8"
);

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
function ok(label, cond) {
  eq(label, !!cond, true);
}

// ---------------------------------------------------------------- 函数抽取
const startMark = "// ---- 数据存储：占用展示与立即整理 ----";
const endMark = "async function loadStorageInfo() {";
const a = appSrc.indexOf(startMark);
const b = appSrc.indexOf(endMark, a);
if (a < 0 || b < 0) throw new Error("抽取失败：数据存储段落");
const code = appSrc.slice(a, b);

const storeEl = { innerHTML: "" };
const $ = (id) => (id === "storageInfo" ? storeEl : null);
const api = new Function(
  "$",
  code + "\n; return { fmtBytes, stgPct, renderStorageInfo };"
)($);

// ------------------------------------------------- 与后端结构体字段名对齐
// 从 Rust 源码里抠出 pub 字段名：这是字段契约的唯一真相源
function structFields(src, name) {
  const m = src.match(new RegExp("pub struct " + name + "\\s*\\{([\\s\\S]*?)\\n\\}"));
  if (!m) throw new Error("在 maintain.rs 中找不到结构体 " + name);
  const out = [];
  m[1].split("\n").forEach(function (line) {
    const f = line.match(/^\s*pub\s+([a-z_][a-z0-9_]*)\s*:/);
    if (f) out.push(f[1]);
  });
  if (!out.length) throw new Error(name + " 未解析到任何字段");
  return out;
}
const infoFields = structFields(rustSrc, "StorageInfo");
const sliceFields = structFields(rustSrc, "StorageSlice");

console.log("== 字段契约（maintain.rs ↔ app.js） ==");
ok("解析到 StorageInfo 字段", infoFields.length >= 8);
ok("字段为 snake_case（无大写字母）", !infoFields.some((f) => /[A-Z]/.test(f)));
ok("字段为 snake_case 的 slice", !sliceFields.some((f) => /[A-Z]/.test(f)));

// 前向：载荷字段必须与结构体完全一致，少写/多写/改名都会在这里炸
const payload = {
  db_bytes: 126976,
  wal_bytes: 4194304,
  icon_files: 74,
  icon_bytes: 143000,
  earliest_date: "2026-08-20",
  retention_days: 0,
  total_bytes: 4464526,
  slices: [],
  approx: false,
};
eq(
  "测试载荷字段与 StorageInfo 一致",
  Object.keys(payload).sort().join(","),
  infoFields.slice().sort().join(",")
);

// 反向：renderStorageInfo 实际读的字段必须真实存在（本次 bug 的守门人）
const accessed = {};
code.replace(/(?:^|[^\w$])info\.([A-Za-z_][A-Za-z0-9_]*)/g, function (_, f) {
  accessed[f] = 1;
  return "";
});
const ghost = Object.keys(accessed).filter((f) => infoFields.indexOf(f) < 0);
eq("renderStorageInfo 读了不存在的字段", ghost.join(",") || "无", "无");

const sAccessed = {};
code.replace(/(?:^|[^\w$])s\.([A-Za-z_][A-Za-z0-9_]*)/g, function (_, f) {
  sAccessed[f] = 1;
  return "";
});
const sGhost = Object.keys(sAccessed).filter((f) => sliceFields.indexOf(f) < 0);
eq("渲染读了 slice 上不存在的字段", sGhost.join(",") || "无", "无");

console.log("== fmtBytes ==");
eq("0 字节", api.fmtBytes(0), "0 B");
eq("不足 1KB", api.fmtBytes(512), "512 B");
eq("1KB 边界", api.fmtBytes(1024), "1.0 KB");
eq("整 KB", api.fmtBytes(2048), "2.0 KB");
eq("1MB 边界", api.fmtBytes(1024 * 1024), "1.00 MB");
eq("混合大小", api.fmtBytes(5 * 1024 * 1024), "5.00 MB");
eq("非法输入兜底 0", api.fmtBytes("abc"), "0 B");

console.log("== stgPct 占比 ==");
eq("四分之一", api.stgPct(50, 200), "25.0%");
eq("三分之二", api.stgPct(1024, 1536), "66.7%");
eq("极小项不显示 0.0%", api.stgPct(1, 100000), "<0.1%");
eq("0 字节", api.stgPct(0, 100), "0%");
eq("总量为 0 不除零", api.stgPct(10, 0), "0%");
eq("非法总量兜底", api.stgPct(10, "x"), "0%");

console.log("== renderStorageInfo 渲染 ==");
storeEl.innerHTML = "";
api.renderStorageInfo(null);
eq("info 为空不渲染", storeEl.innerHTML, "");

const info = Object.assign({}, payload, {
  slices: [
    { key: "wal", label: "写前日志", bytes: 4194304, rows: 0, unit: "" },
    { key: "overtime", label: "加班记录", bytes: 1024, rows: 12, unit: "行" },
    { key: "icons", label: "图标缓存", bytes: 0, rows: 3, unit: "个文件" },
  ],
});
api.renderStorageInfo(info);
const html = storeEl.innerHTML;
ok("显示总计标题", html.indexOf("共占用") >= 0);
ok("显示总计大小", html.indexOf("4.26 MB") >= 0);
ok("显示分类名", html.indexOf("加班记录") >= 0);
ok("显示行数", html.indexOf("12 行") >= 0);
ok("显示占比", html.indexOf("93.9%") >= 0);
eq("0 字节的项被过滤", html.indexOf("图标缓存"), -1);
ok("每行带配色类", html.indexOf('class="stg-dot k-overtime"') >= 0);
ok("条形段带配色类", html.indexOf('class="stg-seg k-wal"') >= 0);
ok("显示最早数据日期", html.indexOf("最早数据 2026-08-20") >= 0);
eq("非估算时不显示估算提示", html.indexOf("估算"), -1);
// 对齐：大小与占比必须是各自独立的定宽列，否则长短数字会参差
eq("大小列存在", html.indexOf('class="stg-size"') >= 0, true);
eq("占比列存在", html.indexOf('class="stg-pct"') >= 0, true);
eq("不再使用挤在一起的老结构", html.indexOf("stg-val"), -1);
eq("每行两个数字列", (html.match(/class="stg-size"/g) || []).length, 2);
eq("每行一个占比列", (html.match(/class="stg-pct"/g) || []).length, 2);

console.log("== 边界 ==");
api.renderStorageInfo({ total_bytes: 0, slices: [] });
eq("空数据提示", storeEl.innerHTML, '<span class="hint">暂无占用数据</span>');

api.renderStorageInfo({
  total_bytes: 100,
  approx: true,
  slices: [{ key: "activity", label: "键鼠活动", bytes: 100, rows: 5, unit: "行" }],
});
ok("估算时给出提示", storeEl.innerHTML.indexOf("按行数比例估算") >= 0);

console.log("== 配色与列对齐样式 ==");
// 后端 maintain.rs 的 GROUP_ORDER + 文件类 key，缺一个就会显示成黑块
const keys = [
  "overtime", "activity", "app", "audio",
  "other", "wal", "icons", "config", "logs",
];
const missCss = keys.filter((k) => css.indexOf(".k-" + k) < 0);
eq("styles.css 中缺失的配色类", missCss.join(",") || "无", "无");

const missRust = keys.filter((k) => rustSrc.indexOf('"' + k + '"') < 0);
eq("maintain.rs 中缺失的 key", missRust.join(",") || "无", "无");

const liRule = (css.match(/\.stg-list li\s*\{[^}]*\}/) || [""])[0];
ok(".stg-list li 用 grid 定宽列", /display:\s*grid/.test(liRule));
ok("grid 里为大小/占比/行数留了固定列", (liRule.match(/\d+px/g) || []).length >= 4);

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
