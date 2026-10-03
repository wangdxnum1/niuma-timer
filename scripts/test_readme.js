// README 双语一致性守卫（v1.4.0 包 D）。
// 事故根因：两份 README 停在 1.1.0，v1.2.0/v1.3.0 两轮功能零文档。本脚本由
// run_all.js 自动收集，build.bat test 与 release.bat 第 0 步必然跑到——
// 只改一份文档、忘了发版改版本号，都会立刻红。
// 运行：node scripts/test_readme.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const en = fs.readFileSync(path.join(ROOT, "README.md"), "utf8");
const zh = fs.readFileSync(path.join(ROOT, "README.zh-CN.md"), "utf8");
const cargo = fs.readFileSync(path.join(ROOT, "src-tauri", "Cargo.toml"), "utf8");

let pass = 0;
let fail = 0;
function eq(label, actual, expect) {
  if (actual === expect) {
    pass++;
    console.log("  PASS " + label + "  ->  " + actual);
  } else {
    fail++;
    console.log("  FAIL " + label + "\n       期望: " + expect + "\n       实际: " + actual);
  }
}

const cargoVer = (cargo.match(/^version = "(\d+\.\d+\.\d+)"/m) || ["", ""])[1];
const boldVer = (s) => {
  const m = s.match(/\*\*(\d+\.\d+\.\d+)\*\*/);
  return m ? m[1] : "";
};
const h2Count = (s) => (s.match(/^## /gm) || []).length;
const cjkCount = (s) => (s.match(/[\u4e00-\u9fff]/g) || []).length;
// 语义锚点：两侧标题语言不同，按「语义等价」配对（以现有真实标题为准）
const ANCHORS = [
  ["Features", "功能特性"],
  ["How it works", "计算原理"],
];

console.log("== 版本号 ==");
eq("英文 README 版本 == Cargo.toml", boldVer(en), cargoVer);
eq("中文 README 版本 == Cargo.toml（两份即相等）", boldVer(zh), cargoVer);

// 版本三处同步守卫（技术债批次三）：此前 Cargo.toml ↔ tauri.conf.json 只在
// release.bat / release.yml 发版当天把关，CHANGELOG 有没有当前版本段更是
// 只有 publish_release.py 的 WARN——忘同步时用户更新弹窗会拿「未发布」内容
// 冒充本版公告。测试守卫让它在 build.bat test / CI 全入口提前红。
const conf = JSON.parse(fs.readFileSync(path.join(ROOT, "src-tauri", "tauri.conf.json"), "utf8"));
const changelog = fs.readFileSync(path.join(ROOT, "CHANGELOG.md"), "utf8");
eq("tauri.conf.json 版本 == Cargo.toml", conf.version, cargoVer);
eq(
  "CHANGELOG 存在当前版本段 ## [" + cargoVer + "]",
  new RegExp("^## \\[" + cargoVer.replace(/\./g, "\\.") + "\\]", "m").test(changelog),
  true
);

console.log("== 结构对齐 ==");
eq("两份 ## 二级标题数量相等", h2Count(en), h2Count(zh));
eq(
  "语义锚点两侧齐备",
  ANCHORS.every(([a, b]) => en.indexOf("## " + a) >= 0 && zh.indexOf("## " + b) >= 0),
  true
);

console.log("== 互链与入口 ==");
eq("英文 README 链中文文档", en.indexOf("README.zh-CN.md") >= 0, true);
eq("中文 README 链英文文档", zh.indexOf("README.md") >= 0, true);
eq("两份都链 CHANGELOG.md", en.indexOf("CHANGELOG.md") >= 0 && zh.indexOf("CHANGELOG.md") >= 0, true);

console.log("== 语言纯度 ==");
eq("中文文档 CJK 字符数 ≥ 500", cjkCount(zh) >= 500, true);
eq("英文文档 CJK 字符数 ≤ 20", cjkCount(en) <= 20, true);

console.log("\n" + pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
