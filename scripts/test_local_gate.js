// 本地发版门禁与云端六步的一致性守卫（技术债第三轮批次二，2026-10-03）。
//
// release.bat 的第 0 步只跑 build.bat test：此前 fmt / clippy / cargo-deny
// 三步只在 tag 推上去后的云端跑，而 tag 不可变——本地全绿、云端红 = 版本号
// 白白消耗，只能人肉 bump 重打。本文件钉住四件事：
//  1. build.bat :do_test 含与 tests.yml 逐字同 flag 的三步（fail-closed）
//  2. workflow 的 install-action 工具钉版（cargo-deny 精确版 / tauri-cli 大版本），
//     防上游发大版本让 CI 隔夜变红而本地依旧绿
//  3. REPO 常量在 release.bat 与 push_via_api.py 两处字面量一致
//  4. release.yml PDB 步骤带 continue-on-error（与 test_publish_release.py 互补）
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const read = (p) => fs.readFileSync(path.join(ROOT, p), "utf8");
let pass = 0, fail = 0;
function eq(label, actual, expected) {
  const ok = actual === expected;
  console.log(`  ${ok ? "PASS" : "FAIL"} ${label}` + (ok ? "" : `  期望 ${expected} 实际 ${actual}`));
  if (ok) pass++; else fail++;
}
function has(label, src, needle) {
  eq(label, src.includes(needle) ? "含" : "缺", "含");
}

const buildBat = read("build.bat");
const releaseYml = read(".github/workflows/release.yml");
const testsYml = read(".github/workflows/tests.yml");
const relBat = read("release.bat");
const pushPy = read("scripts/push_via_api.py");

console.log("== build.bat :do_test 本地门禁对齐云端六步 ==");
has("cargo fmt --all -- --check", buildBat, "%CARGO_BIN% fmt --all -- --check");
has("cargo clippy --all-targets -- -D warnings", buildBat, "%CARGO_BIN% clippy --all-targets -- -D warnings");
has("cargo deny check advisories", buildBat, "deny check advisories");
has("cargo-deny 缺失时 fail-closed 并给安装指引", buildBat, "cargo install cargo-deny --locked");

console.log("== workflow 工具钉版 ==");
has("tests.yml cargo-deny 精确钉版", testsYml, "tool: cargo-deny@0.20.2");
has("release.yml tauri-cli 大版本钉版", releaseYml, "tool: tauri-cli@2");

console.log("== REPO 常量单一事实 ==");
const repoBat = relBat.match(/set "REPO=([^"]+)"/);
const repoPy = pushPy.match(/REPO = "([^"]+)"/);
eq("release.bat REPO", repoBat ? repoBat[1] : "缺失", "wangdxnum1/niuma-timer");
eq("push_via_api.py REPO 与 release.bat 一致", repoPy ? repoPy[1] : "缺失", repoBat ? repoBat[1] : "缺失");

console.log("== release.yml PDB 步骤 best-effort ==");
const pdbStep = (releaseYml.split("Upload PDB (best-effort)")[1] || "").split("- name:")[0];
has("PDB 步骤 continue-on-error: true", pdbStep, "continue-on-error: true");

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
