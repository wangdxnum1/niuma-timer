// A 阶段自动更新断言脚本（零注册：run_all.js 自动收集 scripts/test_*.js）
// 用法：node scripts/test_update.js —— 退出码 0 = 全绿，1 = 有失败
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const read = (p) => fs.readFileSync(path.join(ROOT, p), "utf-8");

let pass = 0, fail = 0;
const eq = (name, a, b) => {
  if (a === b) { pass++; } else { fail++; console.log("FAIL " + name + "  期望 " + JSON.stringify(b) + " 实际 " + JSON.stringify(a)); }
};
const ok = (name, cond) => eq(name, !!cond, true);
const has = (name, hay, needle) => eq(name, typeof hay === "string" && hay.includes(needle), true);
const lacks = (name, hay, needle) => eq(name, !(typeof hay === "string" && hay.includes(needle)), true);

const rsConfig = read("src-tauri/src/config.rs");

// ---- 区块 1：配置字段（A1）----
has("config 有 update_auto_check 字段", rsConfig, "pub update_auto_check: bool,");
has("config 有 update_last_check 字段", rsConfig, "pub update_last_check: Option<String>,");
has("config 有 update_skipped_version 字段", rsConfig, "pub update_skipped_version: Option<String>,");
has("config 有 update_last_run_version 字段", rsConfig, "pub update_last_run_version: Option<String>,");
has("config 有自动更新区注释", rsConfig, "// ---- 自动更新（v1.4.0 阶段 A）----");
has("config 有自动检查 doc", rsConfig, "/// 是否自动检查更新");
has("config last_check 带 serde 默认", rsConfig, '#[serde(default)]\n    pub update_last_check: Option<String>,');
has("config Default 有 auto_check 初始", rsConfig, "update_auto_check: true,");
has("config Default 有 last_check 初始", rsConfig, "update_last_check: None,");
has("config Default 有 skipped 初始", rsConfig, "update_skipped_version: None,");

// ---- 区块 2：build.rs 注入更新说明（A3）----
const rsBuild = read("src-tauri/build.rs");
has("build 监控 CHANGELOG 变化", rsBuild, "cargo:rerun-if-changed=../CHANGELOG.md");
has("build 注入 UPDATE_NOTES", rsBuild, "cargo:rustc-env=UPDATE_NOTES=");
has("build 有 CHANGELOG 抽取函数", rsBuild, "fn changelog_notes(");
has("build 抽取函数按版本匹配段落头", rsBuild, 'let header = format!("## [{version}]");');
has("build 换行转义为字面 \\n", rsBuild, String.raw`.replace('\n', "\\n")`);


// ---- 区块 3：updater 工件签名与静态清单配置（A4）----
const rsCargo = read("src-tauri/Cargo.toml");
const conf = JSON.parse(read("src-tauri/tauri.conf.json"));
const gitignore = read(".gitignore");
const upd = (conf.plugins || {}).updater || {};
eq("conf.bundle.createUpdaterArtifacts 为 true", (conf.bundle || {}).createUpdaterArtifacts, true);
ok("conf.updater.pubkey 已配置且长度合理", typeof upd.pubkey === "string" && upd.pubkey.length > 100);
lacks("conf.updater.pubkey 已替换占位符", upd.pubkey, "REPLACE_WITH_PUBKEY");
ok("conf.updater.endpoints 指向本仓库 latest.json", Array.isArray(upd.endpoints) && upd.endpoints.includes("https://github.com/wangdxnum1/niuma-timer/releases/latest/download/latest.json"));
has("Cargo.toml 含 tauri-plugin-updater", rsCargo, "tauri-plugin-updater");
has("Cargo.toml 含 sha2", rsCargo, "sha2");
ok("Cargo.toml 的 reqwest 启用 blocking", /reqwest\s*=\s*\{[^}]*"blocking"/.test(rsCargo));
has(".gitignore 忽略私钥", gitignore, "*.key");
// ---- 区块 4：--apply-update 助手模式（A5）----
const rsMain = read("src-tauri/src/main.rs");
const rsUpd = read("src-tauri/src/update.rs");
const iParse = rsMain.indexOf("update::parse_helper_args");
const iCrash = rsMain.indexOf("install_crash_log();");
has("main.rs 调用 update::parse_helper_args", rsMain, "update::parse_helper_args");
ok("main.rs 拦截位于 install_crash_log 之前", iParse >= 0 && iCrash >= 0 && iParse < iCrash);
has("main.rs 拦截用 process::exit 不初始化 Tauri", rsMain, "std::process::exit(update::run_helper");
has("update.rs 定义 HelperIo 窄接口", rsUpd, "pub trait HelperIo");
has("update.rs 有 run_helper_flow 状态机", rsUpd, "pub fn run_helper_flow<IO: HelperIo>");
has("update.rs 有 sha256_hex", rsUpd, "pub fn sha256_hex");
has("update.rs 引入 sha2", rsUpd, "use sha2::{Digest, Sha256};");
has("update.rs 有真实 IO 实现", rsUpd, "impl HelperIo for RealHelperIo");
has("update.rs 用 win::process_exe_path 探测进程", rsUpd, "crate::win::process_exe_path");

// ---- 区块 5：托盘更新入口（A6）----
const rsTray = read("src-tauri/src/tray.rs");
has("tray.rs 新增「检查更新」静态项", rsTray, '"check-update", "检查更新"');
has("tray.rs 新增动态项 id", rsTray, '"update-now"');
has("tray.rs 动态项文案含版本", rsTray, 'format!("更新到 v{version}")');
has("tray.rs 有 set_update_available", rsTray, "pub fn set_update_available(app: &AppHandle, version: &str)");
has("tray.rs 记录已挂版本", rsTray, "static UPDATE_VERSION: Mutex<Option<String>>");
has("tray.rs 持有托盘菜单句柄", rsTray, "static TRAY_MENU: Mutex<Option<Menu<tauri::Wry>>>");
has("tray.rs 动态项追加进菜单", rsTray, "menu.append(&item)");
has("tray.rs tooltip 追加新版本提示", rsTray, "（有新版本 v{v}）");
has("tray.rs 托盘项通知前端", rsTray, '"update-view-requested"');
has("tray.rs create_tray 收尾补挂早到版本", rsTray, "let early = sync::lock(&UPDATE_VERSION");

// ---- 区块 6：检查更新四命令 + 安装版判定 + 自动检查调度（A7）----
const rsMain7 = read("src-tauri/src/main.rs");
const rsUpd7 = read("src-tauri/src/update.rs");
const conf7 = read("src-tauri/tauri.conf.json");
has("main.rs 定义 check_update 命令", rsMain7, "fn check_update(");
has("main.rs 定义 start_update 命令", rsMain7, "fn start_update(");
has("main.rs 定义 skip_update_version 命令", rsMain7, "fn skip_update_version(");
has("main.rs 定义 take_update_announcement 命令", rsMain7, "fn take_update_announcement(");
has("四命令已注册进 invoke_handler", rsMain7, "check_update,\n            start_update,\n            skip_update_version,\n            take_update_announcement");
has("main.rs 注册 updater 插件", rsMain7, "tauri_plugin_updater::Builder::new().build()");
has("setup 启动自动检查线程", rsMain7, "update::spawn_update_checker(app.handle().clone())");
has("update.rs 有安装版判定", rsUpd7, "pub fn is_installed() -> bool");
has("安装版判定读卸载表 DisplayName", rsUpd7, 'get_value("DisplayName")');
has("安装版判定包含机器级安装", rsUpd7, "HKEY_LOCAL_MACHINE");
has("安装版判定核对运行路径", rsUpd7, "installation_matches(&exe, &location, &icon)");
has("update.rs 有自动检查线程入口", rsUpd7, "pub fn spawn_update_checker(app: tauri::AppHandle)");
has("update.rs 有 UpdateInfo 返回体", rsUpd7, "pub struct UpdateInfo {");
has("update.rs 有 SHA256SUMS 取哈希", rsUpd7, "pub fn find_hash_in_sums(");
has("update.rs 有刚升级判定", rsUpd7, "pub fn is_just_upgraded(");
const mEp = conf7.match(/"endpoints"\s*:\s*\[\s*"([^"]+)"/);
has("update.rs 默认端点与 tauri.conf 同值", rsUpd7, mEp ? mEp[1] : "___no_endpoint___");

// ---- 区块 7：更新视图 + 设置页开关 + 升级公告（A8）----
const htmlA8 = read("frontend/index.html");
const appA8 = read("frontend/app.js");
const cssA8 = read("frontend/styles.css");
has("index.html 有更新视图", htmlA8, 'id="viewUpdate"');
lacks("更新视图不占侧栏位（rail 无 viewUpdate）", htmlA8, 'data-nav="viewUpdate"');
has("设置页有自动检查更新开关", htmlA8, 'id="update_auto_check"');
has("设置页有检查更新按钮", htmlA8, 'id="settingsCheckUpdateBtn"');
has("更新页有更新说明区", htmlA8, 'id="updNotes"');
has("更新页有进度区", htmlA8, 'id="updProgress"');
has("更新页有立即更新按钮", htmlA8, 'id="updUpdateBtn"');
has("更新页有跳过此版本按钮", htmlA8, 'id="updSkipBtn"');
has("更新页高亮归属「设置」", appA8, 'const railKey = id === "viewUpdate" ? "viewSettings" : navKey;');
has("进更新页时取一次数据", appA8, 'if (id === "viewUpdate") {');
has("load 末尾取升级公告", appA8, 'invoke("take_update_announcement")');
has("readCfg 带上自动检查开关", appA8, 'update_auto_check: $("update_auto_check").checked,');
has("按钮接线 check_update", appA8, 'invoke("check_update")');
has("按钮接线 start_update", appA8, 'invoke("start_update")');
has("按钮接线 skip_update_version", appA8, 'invoke("skip_update_version", { version:');
has("监听托盘更新视图事件", appA8, '"update-view-requested"');
has("styles.css 有更新说明换行样式", cssA8, "white-space: pre-wrap;");

// ---- 区块 8：发布链闭合（latest.json + .sig + 发布校验 + CHANGELOG）（A9）----
const pyA9 = read("scripts/publish_release.py");
const bldA9 = read("build.bat");
const relA9 = read("release.bat");
const chnA9 = read("CHANGELOG.md");
has("publish_release.py 生成 latest.json", pyA9, "latest.json");
has("publish_release.py 复用 CHANGELOG 抽取更新说明", pyA9, "changelog_section(root, version)");
has("publish_release.py 用 UTC 时间作 pub_date", pyA9, "datetime.now(timezone.utc)");
has("publish_release.py 平台键含 windows-x86_64", pyA9, '"windows-x86_64"');
has("publish_release.py 平台键含 windows-x86_64-nsis", pyA9, '"windows-x86_64-nsis"');
has("publish_release.py 把 .sig 纳入上传资产", pyA9, 'endswith(".sig")');
has("publish_release.py 发布后核对 latest.json 引用", pyA9, "not in the release");
has("build.bat 复制 NSIS 签名文件", bldA9, "*.exe.sig");
has("build.bat 复制 MSI 签名文件", bldA9, "*.msi.sig");
has("release.bat 构建前断言签名私钥", relA9, "TAURI_SIGNING_PRIVATE_KEY");
has("release.bat 构建前断言签名口令", relA9, "TAURI_SIGNING_PRIVATE_KEY_PASSWORD");
has("release.bat 发布后校验 latest.json", relA9, "latest.json");
has("release.bat 校验 platforms 非空", relA9, "PSObject.Properties");
has("release.bat 资产清单纳入签名文件", relA9, "*.exe.sig");
has("CHANGELOG 记录自动检查更新", chnA9, "自动检查更新");

console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
