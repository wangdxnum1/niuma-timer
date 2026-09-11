// Tauri 命令 ↔ capabilities 权限清单的一致性检查。
//
// 背景（2026-09-11 事故）：新增 `get_storage_info` / `run_maintenance` 两个命令后
// 忘了往 `capabilities/default.json` 登记，编译照过、后端调度器内部调用也正常
// （内部调用不走 IPC，不受 ACL 限制），只有前端一调就被拒：
//   "Command run_maintenance not allowed by ACL"
// 这类漏登记只有在打开对应界面点一下才会暴露，必须让它在测试里红。
//
// 运行：node scripts/test_commands.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const mainSrc = fs.readFileSync(
  path.join(ROOT, "src-tauri", "src", "main.rs"),
  "utf8"
);
const capPath = path.join(ROOT, "src-tauri", "capabilities", "default.json");
const cap = JSON.parse(fs.readFileSync(capPath, "utf8"));

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

// 跨行匹配：属性与 fn 之间可能夹着别的属性（如 #[allow(...)]）
const re = /#\[(?:tauri::)?command[^\]]*\]\s*(?:(?:#\[[^\]]*\]\s*)|(?:\/\/[^\n]*\n\s*))*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)/g;
const commands = [];
let m;
while ((m = re.exec(mainSrc)) !== null) commands.push(m[1]);

// Rust snake_case -> Tauri 权限名是 kebab-case
const kebab = (s) => s.replace(/_/g, "-");

console.log("== 命令清单 ==");
eq("解析到的命令数 > 0", commands.length > 0, true);

const allowed = new Set(
  cap.permissions.filter((p) => p.startsWith("allow-")).map((p) => p.slice(6))
);

const missing = commands.filter((c) => !allowed.has(kebab(c)));
eq(
  "缺少权限登记的命令",
  missing.length ? missing.map(kebab).join(", ") : "无",
  "无"
);

const stale = [...allowed].filter(
  (a) => !commands.includes(a.replace(/-/g, "_"))
);
eq("已无对应命令的残留权限", stale.length ? stale.join(", ") : "无", "无");

console.log("== 前端 invoke 的命令名 ==");
// 前端 invoke("xxx") 必须都能在后端找到（含同步 / 异步）
const appSrc = fs.readFileSync(path.join(ROOT, "frontend", "app.js"), "utf8");
const inv = new Set();
const ire = /invoke\(\s*"([^"]+)"/g;
while ((m = ire.exec(appSrc)) !== null) inv.add(m[1]);
const unknown = [...inv].filter((n) => !commands.includes(n));
eq("前端调用了但后端不存在的命令", unknown.length ? unknown.join(", ") : "无", "无");

console.log("");
console.log(
  "  共 " + commands.length + " 个命令，permissions " + cap.permissions.length + " 项"
);
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
