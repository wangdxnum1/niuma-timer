// 守护·全局快捷键的回归测试：快捷键注册在 Rust 侧，前端只管设置开关与 hint，
// 这里用源码断言把「配置字段 → apply_shortcuts 注册 → 权限」整条链钉住。
// 用法（任意目录）：node scripts/test_shortcut.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const appSrc = fs.readFileSync(path.join(ROOT, "frontend", "app.js"), "utf8");
const htmlSrc = fs.readFileSync(path.join(ROOT, "frontend", "index.html"), "utf8");
const mainSrc = fs.readFileSync(path.join(ROOT, "src-tauri", "src", "main.rs"), "utf8");
const configSrc = fs.readFileSync(path.join(ROOT, "src-tauri", "src", "config.rs"), "utf8");
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

console.log("== 前端：设置开关与提示 ==");
ok("shortcuts_enabled 开关在守护卡", /id="shortcuts_enabled" type="checkbox" class="switch"/.test(htmlSrc));
ok("hint 含 Alt+Shift+N", /Alt\+Shift\+N 显隐主窗/.test(htmlSrc));
ok("hint 含 Alt+Shift+P", /Alt\+Shift\+P 暂停\/恢复监控/.test(htmlSrc));
ok("load 回填快捷键开关（!== false 缺省 true）", /shortcuts_enabled"\)\.checked = cfg\.shortcuts_enabled !== false/.test(appSrc));
ok("readCfg 采集快捷键开关", /shortcuts_enabled: \$\("shortcuts_enabled"\)\.checked/.test(appSrc));
ok("开关 change 即存（改完 Rust 端即时生效）", /\$\("shortcuts_enabled"\)\.addEventListener\("change", saveNow\)/.test(appSrc));

console.log("== Rust：注册与注销 ==");
ok("apply_shortcuts 函数存在", /fn apply_shortcuts\(app: &tauri::AppHandle, cfg: &config::Config\)/.test(mainSrc));
ok("注册前先 unregister_all", /let _ = mgr\.unregister_all\(\);/.test(mainSrc));
ok("开关关闭直接返回（不注册）", /if !cfg\.shortcuts_enabled \{\s*\n\s*return;/ .test(mainSrc));
ok("注册 Alt+Shift+N", /on_shortcut\("Alt\+Shift\+N"/.test(mainSrc));
ok("注册 Alt+Shift+P", /on_shortcut\("Alt\+Shift\+P"/.test(mainSrc));
ok("N：显→hide", /is_visible\(\)\.unwrap_or\(false\)\s*\{\s*\n\s*let _ = w\.hide\(\);/.test(mainSrc));
ok("N：隐→show+set_focus", /let _ = w\.show\(\);\s*\n\s*let _ = w\.set_focus\(\);/.test(mainSrc));
ok("P：切 toggle_pause", /on_shortcut\("Alt\+Shift\+P"[\s\S]{0,200}toggle_pause\(app\);/.test(mainSrc));
ok("仅 Pressed 时触发（防重复）", /event\.state == ShortcutState::Pressed/.test(mainSrc));
ok("注册失败只记日志不中断", /全局快捷键 Alt\+Shift\+N 注册失败（可能被占用）/.test(mainSrc));

console.log("== Rust：接线与配置 ==");
ok("setup 时应用快捷键", /apply_shortcuts\(app\.handle\(\),/.test(mainSrc));
ok("save_config 后重应用快捷键", /apply_shortcuts\(&app, &merged\);/.test(mainSrc));
ok("global-shortcut 插件已注册", /tauri_plugin_global_shortcut::Builder::new\(\)\.build\(\)/.test(mainSrc));
ok("config.rs 有 shortcuts_enabled 字段", /pub shortcuts_enabled: bool,/.test(configSrc));
ok("config.rs serde 默认 true", /#\[serde\(default = "default_true"\)\]\s*\n\s*pub shortcuts_enabled: bool,/.test(configSrc));
ok("Default impl 含 shortcuts_enabled: true", /shortcuts_enabled: true,/.test(configSrc));
ok("Cargo.toml 依赖 global-shortcut", /tauri-plugin-global-shortcut = "2"/.test(cargoSrc));
ok("merge 白名单含 shortcuts_enabled", /shortcuts_enabled/.test(configSrc));

console.log("\n" + pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
