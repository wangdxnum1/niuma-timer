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
// main.rs 模块化拆分后命令分布在 cmds_*.rs，改走聚合源提取
const { rsSource, rsRead } = require("./lib/rs_sources");
const rsSrc = rsSource();
const mainSrc = rsRead("main.rs");
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

// 跨行匹配：属性与 fn 之间可能夹着别的属性（如 #[allow(...)]）；
// 可见性支持 pub / pub(crate)（命令模块化后统一 pub(crate)）
const re = /#\[(?:tauri::)?command[^\]]*\]\s*(?:(?:#\[[^\]]*\]\s*)|(?:\/\/[^\n]*\n\s*))*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)/g;
const commands = [];
let m;
while ((m = re.exec(rsSrc)) !== null) commands.push(m[1]);

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

console.log("== generate_handler 注册完整性 ==");
// 定义了命令却忘登记进 invoke_handler：编译照过、capabilities 也照写，
// 只有前端 invoke 时报「命令不存在」。拆分后这是新的事故面，必须三方互查。
const handlerBlock = mainSrc.match(/generate_handler!\[([\s\S]*?)\]/);
const handlerNames = handlerBlock
  ? handlerBlock[1]
      .split(",")
      .map((s) => s.trim().split("::").pop().trim())
      .filter((n) => /^[a-z_][a-z0-9_]*$/.test(n))
  : [];
const uniqHandler = [...new Set(handlerNames)];
const unregistered = commands.filter((c) => !uniqHandler.includes(c));
eq(
  "定义了但未注册进 generate_handler 的命令",
  unregistered.length ? unregistered.join(", ") : "无",
  "无"
);
const ghostHandler = uniqHandler.filter((n) => !commands.includes(n));
eq(
  "handler 里没有对应 #[tauri::command] 的幽灵项",
  ghostHandler.length ? ghostHandler.join(", ") : "无",
  "无"
);

console.log("== 前端 invoke 的命令名 ==");
// 前端 invoke("xxx") 必须都能在后端找到（含同步 / 异步）
const appSrc = require("./lib/fe_sources").feSource();
const inv = new Set();
const ire = /invoke\(\s*"([^"]+)"/g;
while ((m = ire.exec(appSrc)) !== null) inv.add(m[1]);
const unknown = [...inv].filter((n) => !commands.includes(n));
eq("前端调用了但后端不存在的命令", unknown.length ? unknown.join(", ") : "无", "无");

console.log("== 慢命令必须 async（约定：同步命令在主线程执行）==");
// 不带 (async) 的 #[tauri::command] 函数体里出现 with_db 即红：SQL 走全局 DB 锁，
// 「立即整理」持锁秒级时同步命令会在主线程等锁，冻结托盘刷新与窗口事件。
// 函数体切到下一个命令属性 / 下一份文档注释 / 测试模块为止。
const syncRe = /#\[tauri::command\]\s*(?:(?:#\[[^\]]*\]\s*)|(?:\/\/[^\n]*\n\s*))*(?:pub(?:\([^)]*\))?\s+)?fn\s+(\w+)/g;
const syncSlow = [];
while ((m = syncRe.exec(rsSrc)) !== null) {
  const bodyStart = m.index + m[0].length;
  const ends = [
    rsSrc.indexOf("#[tauri::command", bodyStart),
    rsSrc.indexOf("\n///", bodyStart),
    rsSrc.indexOf("#[cfg(test)]", bodyStart),
  ].filter((x) => x !== -1);
  const body = rsSrc.slice(bodyStart, ends.length ? Math.min(...ends) : bodyStart + 3000);
  if (/with_db/.test(body)) syncSlow.push(m[1]);
}
eq("同步命令体内不做 DB 查询", syncSlow.length ? syncSlow.join(", ") : "无", "无");

console.log("== get_day_timeline offset 必须钳制 ==");
// offset 是前端 IPC 直达的 i64：只挡负值时极端大值会让 chrono 日期运算 panic、
// 异步命令被任务边界吞掉、invoke 永不 resolve（period_bounds 同类漏网，2026-10-03 收口）。
const tlAt = rsSrc.indexOf("fn get_day_timeline");
const tlEnd = (() => {
  if (tlAt === -1) return -1;
  const next = rsSrc.indexOf("#[tauri::command", tlAt);
  return next === -1 ? tlAt + 2000 : next;
})();
eq(
  "get_day_timeline 用 clamp(0, 1200) 钳制 offset",
  tlAt !== -1 && /offset\.clamp\(0,\s*1200\)/.test(rsSrc.slice(tlAt, tlEnd)) ? "是" : "否",
  "是"
);

console.log("");
console.log(
  "  共 " + commands.length + " 个命令，permissions " + cap.permissions.length + " 项"
);
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
