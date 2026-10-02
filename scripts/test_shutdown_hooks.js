// 退出收口契约：RunEvent::Exit 必须结算所有采集器与状态机。
//
// 事故：focus 段只在「闭合」时落库，而退出路径只调了 activity/app_usage/audio
// 三个 shutdown，focus 漏了 → 托盘工具退出时正在进行的那一段（往往正是当天最长
// 的一段）被静默丢弃，且不留任何日志。
//
// 运行：node scripts/test_shutdown_hooks.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const MAIN = path.join(ROOT, "src-tauri", "src", "main.rs");

let pass = 0;
let fail = 0;
function ok(label, cond) {
  if (cond) {
    pass++;
    console.log("  PASS " + label);
  } else {
    fail++;
    console.log("  FAIL " + label);
  }
}

const src = fs.readFileSync(MAIN, "utf8").replace(/\r\n/g, "\n");

const marker = "tauri::RunEvent::Exit";
const at = src.indexOf(marker);
ok("main.rs 有 RunEvent::Exit 分支", at >= 0);

// 取出 `if let tauri::RunEvent::Exit = event { ... }` 的块体
let block = "";
if (at >= 0) {
  const brace = src.indexOf("{", at);
  let depth = 0;
  let j = brace;
  for (; j < src.length; j++) {
    if (src[j] === "{") depth++;
    else if (src[j] === "}") {
      depth--;
      if (depth === 0) break;
    }
  }
  block = src.slice(brace, j + 1);
}

[
  "activity::shutdown()",
  "app_usage::shutdown()",
  "audio_usage::shutdown()",
  "focus::shutdown(",
].forEach(function (call) {
  ok("退出块调用 " + call, block.indexOf(call) >= 0);
});

ok(
  "focus 结算带配置（阈值口径与 tick 一致）",
  /focus::shutdown\(&\w+/.test(block)
);

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
