// 过渡期字节级还原校验：按 RANGES 从 frontend/app.js 切出期望行，
// 与 frontend/js/<name>.js 逐行比对（CRLF 归一化 + 忽略 FE_VER 值行）。
// app.js 删除后本工具退役（契约由 scripts/test_frontend_split.js 接管）。
//
// 用法：node scripts/lib/split_check.js [chunk.js ...]
//   不带参数 = 校验全部块；带参数 = 只校验指定块。
// 退出码：0 全部通过；1 存在差异或参数非法。
const fs = require("fs");
const path = require("path");

const FRONTEND = path.join(__dirname, "..", "..", "frontend");

// 物理行号区间（1 基闭区间）。键顺序无意义，区间值是 app.js 的物理行号。
// boot.js 由三段非连续区间按出现顺序拼接。
const RANGES = {
  "core.js": [[1, 46]],
  "settings.js": [[47, 316]],
  "hero.js": [[317, 559]],
  "overtime.js": [[560, 901]],
  "monitor.js": [[902, 1553]],
  "bill.js": [[1889, 2132]],
  "insights.js": [[2133, 2506]],
  "storage.js": [[1554, 1786]],
  "update.js": [[2728, 2946]],
  "boot.js": [[1787, 1888], [2507, 2727], [2947, 2972]],
};

// 构建回写会改 FE_VER 的值但不改结构，比对时忽略该值行。
const isFeVerLine = (l) => /^const FE_VER = /.test(l);

function toLines(text) {
  return text.replace(/\r\n/g, "\n").replace(/\n$/, "").split("\n");
}

function main() {
  const appPath = path.join(FRONTEND, "app.js");
  if (!fs.existsSync(appPath)) {
    console.log("frontend/app.js 不存在——拆分已完成，split_check 退役。");
    process.exit(0);
  }
  const appLines = toLines(fs.readFileSync(appPath, "utf8"));

  const requested = process.argv.slice(2);
  const targets = requested.length ? requested : Object.keys(RANGES);
  let fail = false;

  for (const name of targets) {
    const ranges = RANGES[name];
    if (!ranges) {
      console.error(`[error] 未知块名：${name}（合法值：${Object.keys(RANGES).join(", ")}）`);
      fail = true;
      continue;
    }
    const chunkPath = path.join(FRONTEND, "js", name);
    if (!fs.existsSync(chunkPath)) {
      console.log(`[skip] ${name}（尚未切出）`);
      continue;
    }
    const expected = ranges
      .flatMap(([a, b]) => appLines.slice(a - 1, b))
      .filter((l) => !isFeVerLine(l));
    const actual = toLines(fs.readFileSync(chunkPath, "utf8")).filter((l) => !isFeVerLine(l));

    let diffAt = -1;
    for (let i = 0; i < Math.max(expected.length, actual.length); i++) {
      if (expected[i] !== actual[i]) { diffAt = i; break; }
    }
    if (diffAt === -1) {
      console.log(`[ok] ${name}（${actual.length} 行）`);
    } else {
      fail = true;
      console.error(`[FAIL] ${name} 第 ${diffAt + 1} 行不一致`);
      console.error(`  期望: ${JSON.stringify(expected[diffAt])}`);
      console.error(`  实际: ${JSON.stringify(actual[diffAt])}`);
    }
  }
  process.exit(fail ? 1 : 0);
}

main();
