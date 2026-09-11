// 加班手动录入的 IPC 字段契约：
// 前端 invoke 嵌套对象按 serde 原样反序列化（只有顶层命令参数才转 camelCase）。
// 曾经把 cross_midnight 写成 crossMidnight，勾选「下班时间在次日凌晨」永远不生效，
// 编辑一条自动生成的通宵记录还会把标记清掉。
//
// 运行：node scripts/test_ot_input.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const appSrc = fs.readFileSync(path.join(ROOT, "frontend", "app.js"), "utf8");
const rustSrc = fs.readFileSync(
  path.join(ROOT, "src-tauri", "src", "overtime.rs"),
  "utf8"
);

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
function ok(label, cond) {
  eq(label, !!cond, true);
}

function structFields(src, name) {
  const m = src.match(
    new RegExp("pub struct " + name + "\\s*\\{([\\s\\S]*?)\\n\\}")
  );
  if (!m) throw new Error("在 overtime.rs 中找不到结构体 " + name);
  const out = [];
  m[1].split("\n").forEach(function (line) {
    const f = line.match(/^\s*pub\s+([a-z_][a-z0-9_]*)\s*:/);
    if (f) out.push(f[1]);
  });
  if (!out.length) throw new Error(name + " 未解析到任何字段");
  return out;
}

function objectKeys(inner) {
  const keys = [];
  inner.split(",").forEach(function (part) {
    const p = part.trim();
    if (!p) return;
    const named = p.match(/^([A-Za-z_][A-Za-z0-9_]*)\s*:/);
    if (named) {
      keys.push(named[1]);
      return;
    }
    const sh = p.match(/^([A-Za-z_][A-Za-z0-9_]*)$/);
    if (sh) keys.push(sh[1]);
  });
  return keys;
}

const fields = structFields(rustSrc, "ManualOvertimeInput");

const invoke = appSrc.match(
  /invoke\(\s*"save_overtime_record"\s*,\s*\{[\s\S]*?input:\s*\{([^}]+)\}/
);
if (!invoke) throw new Error("找不到 save_overtime_record 的 input 对象");
const jsKeys = objectKeys(invoke[1]);

console.log("== ManualOvertimeInput 字段契约（overtime.rs ↔ app.js） ==");
ok("解析到 ManualOvertimeInput 字段", fields.length >= 4);
ok(
  "结构体字段为 snake_case（无大写字母）",
  !fields.some(function (f) {
    return /[A-Z]/.test(f);
  })
);
eq("前端 input 键数量", jsKeys.length, fields.length);

fields.forEach(function (f) {
  ok("前端发送了 " + f, jsKeys.indexOf(f) >= 0);
});

const extra = jsKeys.filter(function (k) {
  return fields.indexOf(k) < 0;
});
eq(
  "前端多写/写错的字段",
  extra.length ? extra.join(", ") : "无",
  "无"
);
ok(
  "没有 camelCase 幽灵字段 crossMidnight",
  jsKeys.indexOf("crossMidnight") < 0
);

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
