// cargo-deny 包装（技术债第三轮批次二追加，2026-10-04）。
//
// advisory-db 从 github.com 拉取，而本机常态是 github.com 被墙、代理不常开：
// 在线检查会因网络失败把整个门禁卡死。策略：
//   1. 先在线跑 `cargo deny check advisories`（与 CI 同语义）；
//   2. 失败时降级为 `cargo deny --offline check advisories`——但缓存库的
//      最后提交必须 ≤ MAX_AGE_DAYS 天（advisory-db 逐日更新，7 天内可信），
//      缓存过期/缺失仍 fail-closed，提示开代理重跑；
//   3. CI 的 runner 永远在线，走第 1 步即返回，不受影响。
// 真实的 advisory 命中在两种模式下都会失败，不会被兜底掩盖。
const { spawnSync } = require("child_process");
const fs = require("fs");
const os = require("os");
const path = require("path");

const CARGO = process.env.CARGO_BIN || "cargo";
const SRCDIR = path.join(__dirname, "..", "src-tauri");
const MAX_AGE_DAYS = 7;

function run(args) {
  return spawnSync(CARGO, args, {
    cwd: SRCDIR,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
}

// 缓存库目录 ~/.cargo/advisory-dbs/advisory-db-<hash>（目录名随 db URL 哈希变化，
// 可能不止一个）——取「最后提交最新」的那个，其 HEAD 日期即缓存新鲜度。
function newestDbAgeDays() {
  const root = path.join(os.homedir(), ".cargo", "advisory-dbs");
  let best = null;
  try {
    for (const name of fs.readdirSync(root)) {
      if (!name.startsWith("advisory-db-")) continue;
      const dir = path.join(root, name);
      const g = spawnSync("git", ["-C", dir, "log", "-1", "--format=%cs"], {
        encoding: "utf8",
      });
      if (g.status !== 0) continue;
      const d = new Date(g.stdout.trim());
      if (!isNaN(d) && (best === null || d > best)) best = d;
    }
  } catch (_) {
    return Infinity;
  }
  if (best === null) return Infinity;
  return (Date.now() - best.getTime()) / 86400000;
}

const online = run(["deny", "check", "advisories"]);
if (online.status === 0) {
  process.stdout.write(online.stdout);
  console.log("deny: online check passed");
  process.exit(0);
}

const age = newestDbAgeDays();
if (age <= MAX_AGE_DAYS) {
  console.log(
    `[WARN] online advisory-db fetch failed (github.com unreachable?); ` +
      `cached db is ${age.toFixed(1)} days old (<= ${MAX_AGE_DAYS}) - falling back to offline check`
  );
  const offline = run(["deny", "--offline", "check", "advisories"]);
  process.stdout.write(offline.stdout);
  process.stderr.write(offline.stderr);
  if (offline.status === 0) {
    console.log("deny: offline check passed on fresh cached db");
    process.exit(0);
  }
  console.error("deny: offline check FAILED - a real advisory hit; see output above");
  process.exit(offline.status || 1);
}

console.error(
  `deny: online check failed AND cached advisory-db is ${age === Infinity ? "missing" : age.toFixed(1) + " days old"} ` +
    `(limit ${MAX_AGE_DAYS}). Fail-closed: turn on the proxy (or reach github.com) and re-run.`
);
process.exit(1);
