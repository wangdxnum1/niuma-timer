// 数据存储「总占用 + 分类细分」的回归测试：
// 从 app.js 抽出真实的 fmtBytes / stgPct / renderStorageInfo，配桩 DOM 跑断言。
// 运行：node scripts/test_storage.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const appSrc = fs.readFileSync(path.join(ROOT, "frontend", "app.js"), "utf8");
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

const info = {
  totalBytes: 1536,
  dbBytes: 1024,
  walBytes: 512,
  iconFiles: 3,
  iconBytes: 0,
  earliestDate: "2026-09-01",
  approx: false,
  slices: [
    { key: "overtime", label: "加班记录", bytes: 1024, rows: 12, unit: "行" },
    { key: "wal", label: "写前日志", bytes: 512, rows: 0, unit: "" },
    { key: "icons", label: "图标缓存", bytes: 0, rows: 3, unit: "个" },
  ],
};
api.renderStorageInfo(info);
const html = storeEl.innerHTML;
ok("显示总计标题", html.indexOf("共占用") >= 0);
ok("显示总计大小", html.indexOf("1.5 KB") >= 0);
ok("显示分类名", html.indexOf("加班记录") >= 0);
ok("显示行数", html.indexOf("12 行") >= 0);
ok("显示占比", html.indexOf("66.7%") >= 0);
eq("0 字节的项被过滤", html.indexOf("图标缓存"), -1);
ok("每行带配色类", html.indexOf('class="stg-dot k-overtime"') >= 0);
ok("条形段带配色类", html.indexOf('class="stg-seg k-wal"') >= 0);
ok("显示最早数据日期", html.indexOf("最早数据 2026-09-01") >= 0);
eq("非估算时不显示估算提示", html.indexOf("估算"), -1);

console.log("== 边界 ==");
api.renderStorageInfo({ totalBytes: 0, slices: [] });
eq("空数据提示", storeEl.innerHTML, '<span class="hint">暂无占用数据</span>');

api.renderStorageInfo({
  totalBytes: 100,
  approx: true,
  slices: [{ key: "activity", label: "键鼠活动", bytes: 100, rows: 5, unit: "行" }],
});
ok("估算时给出提示", storeEl.innerHTML.indexOf("按行数比例估算") >= 0);

console.log("== 配色与后端 key 一致 ==");
// 后端 maintain.rs 的 GROUP_ORDER + 文件类 key，缺一个就会显示成黑块
const keys = [
  "overtime", "activity", "app", "audio",
  "other", "wal", "icons", "config", "logs",
];
const missCss = keys.filter((k) => css.indexOf(".k-" + k) < 0);
eq("styles.css 中缺失的配色类", missCss.join(",") || "无", "无");

const rustSrc = fs.readFileSync(
  path.join(ROOT, "src-tauri", "src", "maintain.rs"),
  "utf8"
);
const missRust = keys.filter(
  (k) => rustSrc.indexOf('key: "' + k + '"') < 0 && rustSrc.indexOf('"' + k + '"') < 0
);
eq("maintain.rs 中缺失的 key", missRust.join(",") || "无", "无");

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
