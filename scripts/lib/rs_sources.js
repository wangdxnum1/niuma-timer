// Rust 源码聚合器：main.rs 模块化拆分（cmds_*/diag/state）后，
// 替代原先对单文件 src-tauri/src/main.rs 的读取。
// RS_FILES 为 src-tauri/src **递归**下全部 .rs（相对 src/ 的路径，排序即拼接序）。
// 递归是必需的：tray/hover_state.rs 这类子模块曾因只扫顶层而对所有
// rsSource 断言隐形（它那 19 条测试白写了）。
// 注意：presence 类断言（includes / 正则存在性）在聚合文本上语义不变；
// 「代码物理位于哪个文件」或「同文件内先后顺序」敏感的断言，
// 请用 rsRead("main.rs") 单文件读取（参照 test_update.js 的拦截顺序断言）。
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..", "..");
const SRC = path.join(ROOT, "src-tauri", "src");

function walk(dir, prefix, out) {
  fs.readdirSync(dir, { withFileTypes: true })
    .sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))
    .forEach((e) => {
      const rel = prefix ? prefix + "/" + e.name : e.name;
      if (e.isDirectory()) walk(path.join(dir, e.name), rel, out);
      else if (e.name.endsWith(".rs")) out.push(rel);
    });
}

const RS_FILES = [];
walk(SRC, "", RS_FILES);

// 按文件名序拼接全部 Rust 源；块间以单个换行分隔，保证跨文件的正则
// （如 `}\s*fn`）不会把两个文件的结尾与开头粘成一个词法单元。
function rsSource() {
  return RS_FILES.map((f) => fs.readFileSync(path.join(SRC, f), "utf8")).join("\n");
}

function rsRead(name) {
  return fs.readFileSync(path.join(SRC, name), "utf8");
}

module.exports = { RS_FILES, rsSource, rsRead };
