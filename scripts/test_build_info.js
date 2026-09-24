// 构建信息（版本 / 编译时间 / git 提交 / 前端指纹）的回归测试。
//
// 背景：同版本号可以构建很多次，光看 v1.2.0 分不清用户装的是哪一次构建、
// 哪个提交、哪份前端资源。build.rs 把这四项在**编译期**注入 exe，main.rs
// 启动时写进 debug.log / panic.log——用户报问题先看这一行即可对号入座。
//
// 本文件断言这条链路的三个环节都不被改坏：
//   1. build.rs 真的注入了（少了 = 日志里永远是 unknown）
//   2. main.rs 真的写了（注入了却没人输出 = 白注入）
//   3. 取值失败时降级 unknown 而非 panic（源码包无 .git 也要能启动）
//
// 用法（任意目录）：node scripts/test_build_info.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const buildSrc = fs.readFileSync(path.join(ROOT, "src-tauri", "build.rs"), "utf8");
const mainSrc = fs.readFileSync(path.join(ROOT, "src-tauri", "src", "main.rs"), "utf8");
const cargoSrc = fs.readFileSync(path.join(ROOT, "src-tauri", "Cargo.toml"), "utf8");

let pass = 0;
let fail = 0;
function eq(label, actual, expect) {
  if (actual === expect) {
    pass++;
    console.log("  PASS " + label + "  ->  " + actual);
  } else {
    fail++;
    console.log("  FAIL " + label + "\n       期望: " + expect + "\n       实际: " + actual);
  }
}
function ok(label, cond) {
  eq(label, !!cond, true);
}

function main() {
  console.log("== build.rs 编译期注入 ==");
  ok(
    "注入编译时间 BUILD_TIME",
    /cargo:rustc-env=BUILD_TIME=/.test(buildSrc)
  );
  ok(
    "注入 git 提交号 BUILD_GIT",
    /cargo:rustc-env=BUILD_GIT=/.test(buildSrc) && /fn git_rev\(\)/.test(buildSrc)
  );
  ok(
    "注入前端指纹 BUILD_FE_VER（与 ?v= 同源）",
    /cargo:rustc-env=BUILD_FE_VER=\{ver\}/.test(buildSrc)
  );
  ok(
    "git 提交号走 git rev-parse --short HEAD",
    /rev-parse/.test(buildSrc) && /--short/.test(buildSrc) && /HEAD/.test(buildSrc)
  );
  ok(
    "取不到 git 时降级 unknown（源码包/无 git 不致命）",
    /fn git_rev\(\)[\s\S]{0,600}unknown/.test(buildSrc)
  );
  ok(
    "提交后能重新注入：声明 git reflog 触发重跑",
    /cargo:rerun-if-changed=\.\.\/\.git\/logs\/HEAD/.test(buildSrc)
  );
  ok(
    "编译时间取本地时区（chrono::Local）",
    /chrono::Local::now\(\)/.test(buildSrc)
  );
  // 只在 [build-dependencies] 段内找：Cargo.toml 的 [dependencies] 本来就有 chrono，
  // 不加边界的话这条断言会永远为真，测不出 build.rs 到底能不能用
  const buildDeps = (cargoSrc.match(/\[build-dependencies\]([\s\S]*?)(?=\n\[|$)/) || ["", ""])[1];
  ok("build-dependencies 段内有 chrono（build.rs 才用得上）", /chrono/.test(buildDeps));

  console.log("== main.rs 启动落日志 ==");
  ok("定义 build_info() 组装四要素", /fn build_info\(\) -> String/.test(mainSrc));
  ok(
    "版本号取自 CARGO_PKG_VERSION（不硬编码）",
    /env!\("CARGO_PKG_VERSION"\)/.test(mainSrc)
  );
  ok(
    "BUILD_TIME / BUILD_GIT / BUILD_FE_VER 三处都读",
    /BUILD_TIME/.test(mainSrc) && /BUILD_GIT/.test(mainSrc) && /BUILD_FE_VER/.test(mainSrc)
  );
  ok(
    "用 option_env! 兜底 unknown（缺注入也不 panic）",
    /option_env!\("BUILD_TIME"\)/.test(mainSrc) &&
      /option_env!\("BUILD_GIT"\)/.test(mainSrc) &&
      /option_env!\("BUILD_FE_VER"\)/.test(mainSrc)
  );
  ok(
    "启动写 debug.log（日常排查入口）",
    /db::debug_log\(&format!\("build: \{\}", build_info\(\)\)\)/.test(mainSrc)
  );
  ok(
    "启动写 panic.log（双击无反应时确认版本）",
    /trace_startup\(&format!\("build: \{\}", build_info\(\)\)\)/.test(mainSrc)
  );
  // 顺序断言要防 indexOf 双双 -1 的假通过：先各自存在，再比先后
  const logIdx = mainSrc.indexOf('build: {}", build_info()');
  const setupIdx = mainSrc.indexOf(".setup(|app|");
  ok(
    "两条日志都在 setup 之前（webview 起不来也能留痕）",
    logIdx > 0 && setupIdx > 0 && logIdx < setupIdx
  );

  console.log("\n" + pass + " passed, " + fail + " failed");
  process.exit(fail ? 1 : 0);
}

main();
