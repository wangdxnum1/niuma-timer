// 根目录 .bat 的三条契约：
//
// 1) 四个脚本必须**纯 ASCII**（无 >0x7F 字节），且不带 UTF-8 BOM。
//    cmd 会错位解析多字节注释，`chcp 65001` 还会让子进程（vswhere 等）的
//    GBK 输出被误读——两起事故都记在 CHANGELOG 与 build.bat 头部。
// 2) **任何脚本都不得给 RC 赋值**。RC 是工具链环境变量（rc.exe 的路径），
//    embed-resource 一旦看到它就只用它、不再做自己的注册表 / vswhere 发现。
//    2026-10-02 事故：build.bat 拿 RC 当自己的退出码暂存变量，`build.bat debug`
//    跑完把 RC 写成 "0"，紧随其后的 release 阶段就让 embed-resource 去启动 "0"，
//    报 "Are you sure you have RC.EXE in your $PATH or ${RC_$TARGET} or $RC is set?"。
//    症状极具迷惑性：只有**不带参数的 build.bat** 必挂，单独跑 debug 或 release 都正常。
//    现在 common.bat 也不再探测/钉住 RC——上游两条路已足够（vcvars 把 SDK bin
//    放进 PATH；embed-resource 自己走注册表 / vswhere 发现）。
// 3) **TMP / TEMP 只允许 common.bat 赋值**（构建自持的临时目录，治 cl.exe D8050）。
//
// 运行：node scripts/test_bat_vars.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const BATS = ["common.bat", "build.bat", "dev.bat", "release.bat"];
// 谁都不许赋值的工具链变量（embed-resource 会用、且不回退）
const NEVER_ASSIGN = ["RC"];
// 只允许 common.bat 赋值的变量
const ONLY_COMMON = ["TMP", "TEMP"];

let pass = 0;
let fail = 0;
function ok(label, cond, detail) {
  if (cond) {
    pass++;
    console.log("  PASS " + label);
  } else {
    fail++;
    console.log("  FAIL " + label + (detail ? "  ->  " + detail : ""));
  }
}

BATS.forEach(function (name) {
  const p = path.join(ROOT, name);
  const buf = fs.readFileSync(p);
  let bad = 0;
  for (let i = 0; i < buf.length; i++) if (buf[i] > 127) bad++;
  ok(name + " 纯 ASCII", bad === 0, bad + " 个 >0x7F 字节");
  ok(name + " 无 UTF-8 BOM", !(buf[0] === 0xef && buf[1] === 0xbb && buf[2] === 0xbf));

  const src = buf.toString("utf8");
  // 只看赋值语句，不看注释里的提及
  function assignsOf(v) {
    return (src.match(new RegExp("^\\s*set\\s+\"" + v + "=", "gm")) || []).length;
  }

  NEVER_ASSIGN.forEach(function (v) {
    const n = assignsOf(v);
    ok(name + " 不得给工具链变量 " + v + " 赋值", n === 0, n + " 处赋值（会污染工具链环境）");
  });
  ONLY_COMMON.forEach(function (v) {
    const n = assignsOf(v);
    const expect = name === "common.bat" ? 1 : 0;
    ok(name + " 对 " + v + " 的赋值数 = " + expect, n === expect, "实际 " + n);
  });
});

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
