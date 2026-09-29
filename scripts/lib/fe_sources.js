// 前端源码聚合器：替代原先对单文件 frontend/app.js 的读取。
// CHUNKS 为「加载序」（与 frontend/index.html 中 <script> 标签顺序一致）。
// 注意：与 app.js 的物理行序不同——storage 物理在 bill/insights 之前，
// 但按加载序在其后加载（依赖分析结论见 spec 边界原则）。
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..", "..");
const FRONTEND = path.join(ROOT, "frontend");

// 加载序：boot.js 必须最后
const CHUNKS = [
  "core.js", "settings.js", "hero.js", "overtime.js", "monitor.js",
  "bill.js", "insights.js", "storage.js", "update.js", "boot.js",
];

// 按加载序拼接全部前端块。各块文件均以单个换行结尾，
// 用空串拼接即可完整保留每块字节内容（不引入多余空行）。
function feSource() {
  return CHUNKS.map((n) => fs.readFileSync(path.join(FRONTEND, "js", n), "utf8")).join("");
}

function readIndexHtml() {
  return fs.readFileSync(path.join(FRONTEND, "index.html"), "utf8");
}

module.exports = { CHUNKS, feSource, readIndexHtml };
