// 构建脚本的环境契约：`build.bat`（debug→release 连跑）递给工具链的环境里，
// 工具链变量必须仍然可用。
//
// 背景（2026-10-02 事故）：`build.bat` 用 `RC` 当自己的退出码暂存变量，而 `RC`
// 是工具链变量（rc.exe 路径）。debug 那次跑完把 `RC` 写成 "0"，紧接着 release
// 那次就让 embed-resource 去启动 "0"，报一句完全误导人的
// "Are you sure you have RC.EXE in your $PATH or ${RC_$TARGET} or $RC is set?"。
// 症状是「只有不带参数的 build.bat 必挂、单独跑 debug 或 release 都正常」。
//
// 做法：把 CARGO_BIN 指向一个桩（`call 桩.bat`，这样 build.bat 还能接着往下跑），
// 桩把每次被调用时看到的环境记到日志；随后断言两个 flavor 拿到的 RC/TMP 都正常。
// 不编任何代码，秒级完成。
//
// 运行：node scripts/test_build_env.js   （非 Windows 平台自动跳过）
const fs = require("fs");
const path = require("path");
const { spawnSync } = require("child_process");

const ROOT = path.join(__dirname, "..");

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

if (process.platform !== "win32") {
  console.log("  [skip] 构建脚本只在 Windows 上有意义（cmd / rc.exe）");
  console.log("");
  console.log("0 passed, 0 failed (skipped)");
  process.exit(0);
}

const TMPDIR = path.join(ROOT, ".tmp");
fs.mkdirSync(TMPDIR, { recursive: true });
const STUB = path.join(TMPDIR, "stubcargo.bat");
const LOG = path.join(TMPDIR, "build-env.log");

// 桩：记录本次调用看到的 RC/TMP，并伪造出 build.bat 下一步要拷的 exe，
// 好让它继续跑到下一个 flavor。
const STUB_SRC = [
  "@echo off",
  'set "FL=debug"',
  'echo %* | findstr /C:"--release" >nul && set "FL=release"',
  '>>"%BUILD_ENV_LOG%" echo FLAVOR=%FL% RC=[%RC%] TMP=[%TMP%]',
  'if not exist "%~dp0..\\src-tauri\\target\\%FL%" mkdir "%~dp0..\\src-tauri\\target\\%FL%"',
  'echo stub > "%~dp0..\\src-tauri\\target\\%FL%\\niuma-timer.exe"',
  "exit /b 0",
  "",
].join("\r\n");
fs.writeFileSync(STUB, STUB_SRC, "ascii");
if (fs.existsSync(LOG)) fs.unlinkSync(LOG);

// 命令行写进一个临时 bat 再执行：node 的 spawnSync 会给含空格的参数加引号，
// 与 cmd 的 `set "VAR=值"` 引号嵌套后会被解析坏（实测 stub 一次都没被调到）。
const RUNNER = path.join(TMPDIR, "run-build-env.bat");
fs.writeFileSync(
  RUNNER,
  [
    "@echo off",
    'set "CARGO_BIN=call ' + STUB + '"',
    'set "BUILD_ENV_LOG=' + LOG + '"',
    'cd /d "' + ROOT + '"',
    "call build.bat",
    "",
  ].join("\r\n"),
  "ascii"
);
// stdio 必须 inherit/ignore：沙箱下 node 的管道 stdio 会被拒（EPERM）
const r = spawnSync("cmd", ["/c", RUNNER], { cwd: ROOT, stdio: "inherit" });
ok("build.bat（无参数，debug→release）整体退出码 0", r.status === 0, "exit=" + r.status);

const lines = fs.existsSync(LOG)
  ? fs.readFileSync(LOG, "utf8").split(/\r?\n/).filter((l) => l.trim())
  : [];

ok("两个 flavor 都真的调到了 cargo（共 2 次）", lines.length === 2, "实际 " + lines.length + " 次");
lines.forEach((l, i) => console.log("       [" + (i + 1) + "] " + l));

// 关键断言：每一次调用看到的 RC 要么为空、要么指向一个真实存在的 rc.exe。
// 修复前第二行会是 RC=[0]。
lines.forEach(function (l, i) {
  const m = l.match(/RC=\[(.*)\] TMP=\[(.*)\]/);
  if (!m) {
    ok("第 " + (i + 1) + " 次调用可解析", false, l);
    return;
  }
  const rc = m[1];
  const tmp = m[2];
  const rcOk = rc === "" || (fs.existsSync(rc) && /(^|[\\/])rc\.exe$/i.test(rc));
  ok(
    "第 " + (i + 1) + " 次调用：RC 为空或真实 rc.exe",
    rcOk,
    "RC=[" + rc + "]（工具链变量被自己的脚本污染了？）"
  );
  ok(
    "第 " + (i + 1) + " 次调用：TMP 是构建自持的 .tmp",
    tmp === TMPDIR,
    "TMP=[" + tmp + "]"
  );
});

for (const f of [STUB, RUNNER]) {
  try {
    fs.unlinkSync(f);
  } catch (e) {
    /* 清理失败不影响结论 */
  }
}

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
