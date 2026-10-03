// 前端拆分契约测试（多 script 结构守卫），5 项检查：
//  1. index.html 按 CHUNKS 加载序引用 js/<name>?v=
//  2. app.js 已删除且 frontend/ 内无残留引用
//  3. const FE_VER 仅在 core.js 声明一次
//  4. 非 boot 块顶层仅允许声明与纯挂载（声明-挂载白名单，正则近似）
//  5. js/ 目录 .js 文件集合 == CHUNKS（双向，新 chunk 漏登记立刻红）
const fs = require("fs");
const path = require("path");
const { CHUNKS, readIndexHtml } = require("./lib/fe_sources");

const FRONTEND = path.join(__dirname, "..", "frontend");
const JS_DIR = path.join(FRONTEND, "js");

function walk(dir, exts, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) walk(p, exts, out);
    else if (exts.includes(path.extname(e.name))) out.push(p);
  }
  return out;
}

const failures = [];

// -- 1. index.html 标签按加载序排列 ----------------------------------------
const html = readIndexHtml();
let pos = -1;
for (const name of CHUNKS) {
  const needle = `js/${name}?v=`;
  const at = html.indexOf(needle);
  if (at === -1) {
    failures.push(`index.html 缺少 <script src="${needle}...">（CHUNKS 加载序要求全部 10 个标签）`);
  } else if (at < pos) {
    failures.push(`index.html 中 ${needle} 出现在前一个标签之前（违反加载序）`);
  } else {
    pos = at;
  }
}

// -- 2. app.js 已删除且无残留引用 ------------------------------------------
if (fs.existsSync(path.join(FRONTEND, "app.js"))) {
  failures.push("frontend/app.js 仍存在——拆分契约要求删除单文件入口");
}
for (const f of walk(FRONTEND, [".html", ".css", ".js"])) {
  if (f.replace(/\\/g, "/").endsWith("frontend/app.js")) continue;
  const lines = fs.readFileSync(f, "utf8").split("\n");
  lines.forEach((line, i) => {
    if (/app\.js/.test(line)) {
      failures.push(`${path.relative(FRONTEND, f)}:${i + 1} 残留 app.js 引用：${line.trim()}`);
    }
  });
}

// -- 3. FE_VER 仅在 core.js ------------------------------------------------
const corePath = path.join(JS_DIR, "core.js");
if (!fs.existsSync(corePath)) {
  failures.push("frontend/js/core.js 不存在（FE_VER 必须在 core.js 声明）");
} else {
  for (const name of CHUNKS) {
    const p = path.join(JS_DIR, name);
    if (!fs.existsSync(p)) continue;
    const hits = fs.readFileSync(p, "utf8").split("\n")
      .filter((l) => /^const FE_VER = /.test(l)).length;
    if (name === "core.js") {
      if (hits !== 1) failures.push(`core.js 应声明且仅声明一次 const FE_VER（当前 ${hits} 处）`);
    } else if (hits > 0) {
      failures.push(`${name} 不允许声明 const FE_VER（仅 core.js 可声明）`);
    }
  }
}

// -- 4. 非 boot 块顶层白名单（声明-挂载） ------------------------------------
// 顶层（列 0）非空行仅允许：声明（function/async function/const/let/var）、
// 纯挂载（$、document.*、window.*）、块/数组字面量续行（}/)/]/[）、注释。裸业务调用即红。
const TOP_OK = /^(function\b|async function\b|const\b|let\b|var\b|\$|document\.|window\.|\}|\)|\]|\[|\/\/|\/\*| \*|\*\/)/;
for (const name of CHUNKS.slice(0, -1)) {
  const p = path.join(JS_DIR, name);
  if (!fs.existsSync(p)) continue;
  const bad = [];
  fs.readFileSync(p, "utf8").replace(/\r/g, "").split("\n").forEach((line, i) => {
    if (line.length > 0 && !/^[ \t]/.test(line) && !TOP_OK.test(line)) bad.push(`  L${i + 1}: ${line.trim().slice(0, 80)}`);
  });
  if (bad.length) failures.push(`${name} 顶层出现白名单之外的语句：\n${bad.join("\n")}`);
}

// -- 5. 缓存戳一致性：index.html 里全部 ?v= 与 core.js 的 FE_VER 必须同值 -----
// build.rs 每次构建统一回写这 11 处（1 CSS + 10 JS）；一致性此前完全托付
// build.rs，测试不校验——手工误改其中一处（或漏 bump 提交）会静默吃旧缓存，
// 守卫的红只能由本检查给出。
const coreSrc = fs.readFileSync(path.join(JS_DIR, "core.js"), "utf8");
// FE_VER 带 v 前缀（"v86d7b7a0"），URL 参数不带（"?v=86d7b7a0"）——比较前剥掉
const feVer = ((coreSrc.match(/^const FE_VER = "([^"]+)"/m) || [])[1] || "").replace(/^v/, "");
if (!feVer) {
  failures.push("core.js 缺少 const FE_VER 声明");
} else {
  const stamps = [...html.matchAll(/\?v=([A-Za-z0-9_-]+)/g)].map((m) => m[1]);
  if (stamps.length !== CHUNKS.length + 1) {
    failures.push(
      `index.html 应有 ${CHUNKS.length + 1} 处 ?v=（1 CSS + ${CHUNKS.length} JS），实际 ${stamps.length}`
    );
  }
  for (const [i, v] of stamps.entries()) {
    if (v !== feVer) {
      failures.push(`index.html 第 ${i + 1} 处缓存戳 ?v=${v} != FE_VER(${feVer})——改前端后需 cargo build 让 build.rs 统一回写`);
    }
  }
}

// -- 5. js/ 目录 .js 文件集合 == CHUNKS（双向）--------------------------------
// 新 chunk 漏登记 fe_sources.js 时会静默逃出加载序检查与 feSource 聚合测试。
const onDisk = walk(JS_DIR, [".js"])
  .map((f) => path.relative(JS_DIR, f).replace(/\\/g, "/"))
  .sort();
const listed = [...CHUNKS].sort();
const extra = onDisk.filter((f) => !listed.includes(f));
const missing = listed.filter((f) => !onDisk.includes(f));
if (extra.length) failures.push(`frontend/js/ 有未登记进 CHUNKS 的文件：${extra.join(", ")}`);
if (missing.length) failures.push(`CHUNKS 引用了但 js/ 目录不存在的文件：${missing.join(", ")}`);

if (failures.length) {
  console.error(`前端拆分契约检查失败（${failures.length} 项）：`);
  for (const f of failures) console.error(`- ${f}`);
  process.exit(1);
}
console.log(`前端拆分契约检查通过（${CHUNKS.length} 块）`);
