// Rust 源码聚合器：main.rs 模块化拆分（cmds_*/diag/state）后，
// 替代原先对单文件 src-tauri/src/main.rs 的读取。
// RS_FILES 为 src-tauri/src 顶层全部 .rs（文件名排序即拼接序）。
// 注意：presence 类断言（includes / 正则存在性）在聚合文本上语义不变；
// 「代码物理位于哪个文件」或「同文件内先后顺序」敏感的断言，
// 请用 rsRead("main.rs") 单文件读取（参照 test_update.js 的拦截顺序断言）。
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..", "..");
const SRC = path.join(ROOT, "src-tauri", "src");

const RS_FILES = fs
  .readdirSync(SRC)
  .filter((f) => f.endsWith(".rs"))
  .sort();

// 按文件名序拼接全部 Rust 源；块间以单个换行分隔，保证跨文件的正则
// （如 `}\s*fn`）不会把两个文件的结尾与开头粘成一个词法单元。
function rsSource() {
  return RS_FILES.map((f) => fs.readFileSync(path.join(SRC, f), "utf8")).join("\n");
}

function rsRead(name) {
  return fs.readFileSync(path.join(SRC, name), "utf8");
}

module.exports = { RS_FILES, rsSource, rsRead };
