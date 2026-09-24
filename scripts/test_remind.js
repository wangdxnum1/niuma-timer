// 守护·提醒系统通知化的回归测试：横幅机制已整体移除，本文件断言「不再有」，
// 同时兜住通知单通道契约、命令增删、调试卡接线与后端触发契约（冷却/idle/配置默认值）。
// 用法（任意目录）：node scripts/test_remind.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const appSrc = fs.readFileSync(path.join(ROOT, "frontend", "app.js"), "utf8");
const htmlSrc = fs.readFileSync(path.join(ROOT, "frontend", "index.html"), "utf8");
const cssSrc = fs.readFileSync(path.join(ROOT, "frontend", "styles.css"), "utf8");
const capSrc = fs.readFileSync(
  path.join(ROOT, "src-tauri", "capabilities", "default.json"),
  "utf8"
);
const remindSrc = fs.readFileSync(path.join(ROOT, "src-tauri", "src", "remind.rs"), "utf8");
const schedSrc = fs.readFileSync(path.join(ROOT, "src-tauri", "src", "scheduler.rs"), "utf8");
const configSrc = fs.readFileSync(path.join(ROOT, "src-tauri", "src", "config.rs"), "utf8");
const mainSrc = fs.readFileSync(path.join(ROOT, "src-tauri", "src", "main.rs"), "utf8");

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

async function main() {
  console.log("== 横幅机制已移除（前端） ==");
  ok("app.js 不再引用 remind-banner", !/remind-banner/.test(appSrc));
  ok("app.js 无横幅函数（showRemindBar/hideRemindBar）", !/function (show|hide)RemindBar\(/.test(appSrc));
  ok("app.js 无旧横幅按钮接线（remindOk/remindRest）", !/\$\("remindOk"\)|\$\("remindRest"\)/.test(appSrc));
  ok("index.html 无横幅 DOM（remindBar/remindMsg）", !/id="remindBar"|id="remindMsg"/.test(htmlSrc));
  ok("styles.css 无 .remind-bar", !/\.remind-bar/.test(cssSrc));

  console.log("== 提醒统一走系统通知（后端） ==");
  ok("remind.rs 不再 emit remind-banner", !/emit\("remind-banner"/.test(remindSrc));
  ok("remind.rs notify 已简化为 (app, title, body)", /fn notify\(app: &tauri::AppHandle, title: &str, body: &str\)/.test(remindSrc));
  ok("remind.rs notify 走 notification 插件", /notification\(\)/.test(remindSrc));
  ok("remind.rs ack 已删（触发瞬间已重置，ack 冗余）", !/pub fn ack\(\)/.test(remindSrc));

  console.log("== 命令增删（main.rs） ==");
  ok("remind_ack 命令已删", !/remind_ack/.test(mainSrc));
  ok("pause_rest 命令已删", !/pause_rest/.test(mainSrc));
  ok("test_offwork_notify 命令已定义", /fn test_offwork_notify\(app: tauri::AppHandle\)/.test(mainSrc));
  ok("test_sedentary_notify 命令已定义", /fn test_sedentary_notify\(app: tauri::AppHandle\)/.test(mainSrc));
  ok("reset_remind_state 命令已定义", /fn reset_remind_state\(\)/.test(mainSrc));
  ok("run_remind_tick 命令已定义", /fn run_remind_tick\(app: tauri::AppHandle\)/.test(mainSrc));
  ok(
    "test_sedentary_trigger 命令已定义",
    /fn test_sedentary_trigger\(app: tauri::AppHandle\)/.test(mainSrc)
  );
  ok("注册表含 test_offwork_notify", /^\s*test_offwork_notify,$/m.test(mainSrc));
  ok("注册表含 test_sedentary_notify", /^\s*test_sedentary_notify,$/m.test(mainSrc));
  ok("注册表含 test_sedentary_trigger", /^\s*test_sedentary_trigger,$/m.test(mainSrc));
  ok("注册表含 reset_remind_state", /^\s*reset_remind_state,$/m.test(mainSrc));
  ok("注册表含 run_remind_tick", /^\s*run_remind_tick,$/m.test(mainSrc));
  ok("注册表仍含 pause_monitor（手动暂停入口）", /^\s*pause_monitor,$/m.test(mainSrc));

  console.log("== 文案共用（真实触发与调试按钮同源） ==");
  ok("remind.rs 抽出 offwork_texts", /pub\(crate\) fn offwork_texts\(earned: f64\) -> \(String, String\)/.test(remindSrc));
  ok("remind.rs 抽出 sedentary_texts", /pub\(crate\) fn sedentary_texts\(minutes: i64\) -> \(String, String\)/.test(remindSrc));
  ok("remind.rs 抽出 reset_state", /pub\(crate\) fn reset_state\(\)/.test(remindSrc));
  ok("真实下班触发走 offwork_texts", /let \(title, body\) = offwork_texts\(st\.earned\)/.test(remindSrc));
  ok("真实久坐触发走 sedentary_texts", /let \(title, body\) = sedentary_texts\(minutes\)/.test(remindSrc));

  console.log("== capabilities ==");
  ok("无 allow-remind-ack", !/"allow-remind-ack"/.test(capSrc));
  ok("无 allow-pause-rest", !/"allow-pause-rest"/.test(capSrc));
  ok("有 allow-test-offwork-notify", /"allow-test-offwork-notify"/.test(capSrc));
  ok("有 allow-test-sedentary-notify", /"allow-test-sedentary-notify"/.test(capSrc));
  ok("有 allow-test-sedentary-trigger", /"allow-test-sedentary-trigger"/.test(capSrc));
  ok("有 allow-reset-remind-state", /"allow-reset-remind-state"/.test(capSrc));
  ok("有 allow-run-remind-tick", /"allow-run-remind-tick"/.test(capSrc));
  ok("保留 notification:default", /"notification:default"/.test(capSrc));

  console.log("== 调试卡（彩蛋驱动） ==");
  ok("调试卡 DOM 存在且默认 hidden", /id="debugCard" class="debug-card hidden"/.test(htmlSrc));
  ok("调试卡终端风格（标题栏 + 提示行）", /class="debug-head"/.test(htmlSrc) && /class="debug-hint"/.test(htmlSrc));
  ok(
    "五个调试按钮齐全（下班/休息/模拟久坐/重置/立即调度）",
    ["testOffworkBtn", "testSedentaryBtn", "testSedentaryTriggerBtn", "testResetBtn", "testTickBtn"].every(
      (id) => htmlSrc.includes(`id="${id}"`)
    )
  );
  ok(
    "按钮接线 → 五个命令",
    /bindDebugBtn\("testOffworkBtn", "test_offwork_notify"/.test(appSrc) &&
      /bindDebugBtn\("testSedentaryBtn", "test_sedentary_notify"/.test(appSrc) &&
      /bindDebugBtn\("testSedentaryTriggerBtn", "test_sedentary_trigger"/.test(appSrc) &&
      /bindDebugBtn\("testResetBtn", "reset_remind_state"/.test(appSrc) &&
      /bindDebugBtn\("testTickBtn", "run_remind_tick"/.test(appSrc)
  );
  ok(
    "调试按钮失败不再静默（flog 落日志）",
    /function bindDebugBtn[\s\S]{0,400}flog\(/.test(appSrc)
  );
  ok(
    "彩蛋计数器：viewSettings 判定 + eggClicks 自增 + 2s 窗口",
    /nav === "viewSettings"/.test(appSrc) &&
      /eggClicks\+\+/.test(appSrc) &&
      /setTimeout\(\(\) => \(eggClicks = 0\), 2000\)/.test(appSrc)
  );
  ok("第 5 击触发 toggleDebug", /eggClicks >= 5[\s\S]{0,60}toggleDebug\(\)/.test(appSrc));
  ok(
    "toggleDebug：显隐 + toast",
    /function toggleDebug\(\)[\s\S]{0,200}classList\.toggle\("hidden"[\s\S]{0,400}showToast\(/.test(appSrc)
  );
  ok("toast 文案：已开启/已关闭", /调试模式已开启/.test(appSrc) && /调试模式已关闭/.test(appSrc));
  // 回归：.debug-card 带 overflow:hidden 且是限高纵向 flex（.app）的子项，
  // 缺 flex-shrink:0 时会被压成 0 高度（hidden 已移除却看不见）
  ok(
    "调试卡 flex-shrink:0（防在 .app 里被压成 0 高度）",
    /\.debug-card\s*\{[^}]*flex-shrink:\s*0/.test(cssSrc)
  );
  ok(
    "调试卡开启时滚入视野（长设置页底部，仅显隐看不见）",
    /function toggleDebug\(\)[\s\S]{0,260}scrollIntoView\(/.test(appSrc)
  );

  // 「模拟久坐」：真实触发链路要「连续活跃 ≥ 阈值」，阈值最小 1 分钟、tick 又是 60 秒
  // 一拍，手工复测至少等 1 分钟且测不到 50 分钟档的文案。该命令只把「连续活跃起点」
  // 往前拨（外加清冷却），随后照常跑真实 tick——判定/文案/通知/状态重置全走真实代码，
  // 判定函数本身零特判，所以测出来的就是真链路，只是不用干等。
  console.log("== 调试卡·模拟久坐（秒级触发真实链路） ==");
  ok(
    "remind.rs 抽出 force_sedentary_since",
    /pub\(crate\) fn force_sedentary_since\(minutes: i64\)/.test(remindSrc)
  );
  ok(
    "只前拨连续活跃起点 + 清冷却（不动判定逻辑）",
    /fn force_sedentary_since\(minutes: i64\)[\s\S]{0,400}SED_SINCE[\s\S]{0,200}LAST_SED_REMIND/.test(
      remindSrc
    )
  );
  ok(
    "前拨量 = 分钟 × 60000（与判定阈值同口径）",
    /fn force_sedentary_since\(minutes: i64\)[\s\S]{0,300}60_000/.test(remindSrc)
  );
  ok(
    "命令先伪造起点再跑真实 tick",
    /fn test_sedentary_trigger\(app: tauri::AppHandle\)[\s\S]{0,400}force_sedentary_since[\s\S]{0,200}remind::tick\(&app\)/.test(
      mainSrc
    )
  );
  ok("模拟久坐取当前配置阈值（文案里的分钟数才真实）", /fn test_sedentary_trigger[\s\S]{0,200}load_config/.test(mainSrc));

  console.log("== 后端触发契约（不变） ==");
  ok("scheduler 60s 拍挂 remind::tick", /remind::tick\(&?app\)/.test(schedSrc));
  ok("久坐冷却 5 分钟", /REMIND_COOLDOWN_MS: i64 = 5 \* 60 \* 1000/.test(remindSrc));
  ok("idle 满 5 分钟视为已休息", /IDLE_REST_MS: i64 = 5 \* 60 \* 1000/.test(remindSrc));
  ok(
    "config: 久坐开关默认 true",
    /remind_sedentary_enabled: bool,/.test(configSrc) &&
      /remind_sedentary_enabled: true,/.test(configSrc)
  );
  ok("config: 阈值默认 50", /remind_sedentary_minutes: 50,/.test(configSrc));
  ok("config: 下班开关默认 true", /remind_offwork_enabled: true,/.test(configSrc));
  ok("下班提醒开关在设置卡", /id="remind_offwork_enabled"/.test(htmlSrc));

  console.log("\n" + pass + " passed, " + fail + " failed");
  process.exit(fail ? 1 : 0);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
