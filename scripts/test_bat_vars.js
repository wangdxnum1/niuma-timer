// 根目录 .bat 的两条契约：
//
// 1) 四个脚本必须**纯 ASCII**（无 >0x7F 字节）。cmd 会错位解析多字节注释，
//    而 `chcp 65001` 会让子进程（vswhere 等）的 GBK 输出被误读——两起事故都
//    记在 CHANGELOG 与 build.bat 头部。
// 2) 除 common.bat 外，**不得给 RC 赋值**。RC 是工具链环境变量（rc.exe 的路径），
//    embed-resource 一旦看到它就只用它、不再做自己的注册表 / vswhere 发现。
//    2026-10-02 事故：build.bat 拿 RC 当自己的 errorlevel 暂存变量，
//    `build.bat debug` 跑完把 RC 写成 "0"，紧随其后的 release 阶段让
//    embed-resource 去启动 "0"，报
//    "Are you sure you have RC.EXE in your $PATH or ${RC_$TARGET} or $RC is set?"。
//    症状极具迷惑性：`build.bat`（debug→release）必挂，`build.bat release` 单独跑却成功。
//
// 运行：node scripts/test_bat_vars.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const BATS = ["common.bat", "build.bat", "dev.bat", "release.bat"];
// 工具链变量：只允许 common.bat 设置，其它脚本碰它就会毁掉 embed-resource
const TOOLCHAIN_VARS = ["RC"];

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

BATS.forEach(function (name) {
  const p = path.join(ROOT, name);
  const buf = fs.readFileSync(p);
  const bad = [];
  for (let i = 0; i < buf.length; i++) if (buf[i] > 127) bad.push(i);
  ok(
    name + " 纯 ASCII",
    bad.length === 0 ? "无" : bad.length + " 个 >0x7F 字节"
  );
  ok(name + " 无 UTF-8 BOM", !(buf[0] === 0xef && buf[1] === 0xbb && buf[2] === 0xbf));

  const src = buf.toString("utf8");
  TOOLCHAIN_VARS.forEach(function (v) {
    // 只看赋值，不看注释里的提及
    const assigns = (src.match(new RegExp('^\\s*set\\s+"' + v + '=', "gm")) || []).length;
    const expect = name === "common.bat" ? 1 : 0;
    ok(
      name + " 对工具链变量 " + v + " 的赋值数 = " + expect,
      assigns === expect ? String(assigns) : assigns + "（会污染工具链环境）"
    );
  });
});

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
