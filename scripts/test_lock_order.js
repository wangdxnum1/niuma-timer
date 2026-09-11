// 锁顺序契约：禁止在 with_db 回调里调用 ensure_icon / cached_icon。
//
// 调度器 tick 的 settle() 是 ICON_CACHE → DB；IPC summary 若是 DB → ICON_CACHE，
// 新应用首次出现时两条路径交错即 ABBA 死锁——托盘停刷、加班/活动全部停写。
//
// 运行：node scripts/test_lock_order.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const FILES = [
  path.join(ROOT, "src-tauri", "src", "app_usage.rs"),
  path.join(ROOT, "src-tauri", "src", "audio_usage.rs"),
];
const FORBIDDEN = ["ensure_icon", "cached_icon"];

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

function withDbBodies(src) {
  const bodies = [];
  const needle = "with_db(";
  let from = 0;
  while (true) {
    const i = src.indexOf(needle, from);
    if (i < 0) break;
    const brace = src.indexOf("{", i);
    if (brace < 0) break;
    let depth = 0;
    let j = brace;
    for (; j < src.length; j++) {
      if (src[j] === "{") depth++;
      else if (src[j] === "}") {
        depth--;
        if (depth === 0) break;
      }
    }
    bodies.push({ start: brace, body: src.slice(brace, j + 1) });
    from = j + 1;
  }
  return bodies;
}

console.log("== with_db 回调不得持锁提取图标 ==");
FILES.forEach(function (file) {
  const rel = path.relative(ROOT, file);
  const src = fs.readFileSync(file, "utf8");
  const bodies = withDbBodies(src);
  eq(rel + " 解析到 with_db", bodies.length > 0, true);
  bodies.forEach(function (b, idx) {
    FORBIDDEN.forEach(function (fn) {
      const hit = b.body.indexOf(fn) >= 0;
      eq(rel + " with_db#" + (idx + 1) + " 不含 " + fn, hit ? "命中" : "无", "无");
    });
  });
});

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
