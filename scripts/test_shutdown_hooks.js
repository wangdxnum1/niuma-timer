// éåºæ¶å£å¥çº¦ï¼RunEvent::Exit å¿é¡»ç»ç®ææééå¨ä¸ç¶ææºã
//
// äºæï¼focus æ®µåªå¨ãé­åãæ¶è½åºï¼èéåºè·¯å¾åªè°äº activity/app_usage/audio
// ä¸ä¸ª shutdownï¼focus æ¼äº â æçå·¥å·éåºæ¶æ­£å¨è¿è¡çé£ä¸æ®µï¼å¾å¾
// æ­£æ¯å½å¤©æé¿çä¸æ®µï¼è¢«éé»ä¸¢å¼ä¸ä¸çæ¥å¿ã
//
// è¿è¡ï¼node scripts/test_shutdown_hooks.js
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
ok("main.rs æ RunEvent::Exit åæ¯", at >= 0);

// ååº `if let tauri::RunEvent::Exit = event { ... }` çåä½
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
  ok("éåºåè°ç¨ " + call, block.indexOf(call) >= 0);
});

ok(
  "focus ç»ç®å¸¦éç½®ï¼éå¼å£å¾ä¸ tick ä¸è´ï¼",
  /focus::shutdown\(&\w+/.test(block)
);

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
