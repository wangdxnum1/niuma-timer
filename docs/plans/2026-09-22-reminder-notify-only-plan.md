# 提醒纯系统通知化 + 调试模式 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 按已批准设计文档砍掉整套横幅机制（久坐 + 下班提醒统一走系统通知）、砍掉「休息 5 分钟」定时休息链路、新增 config.json `debug` 字段驱动的「调试」卡（测试下班提醒按钮）。

**Architecture:** 后端 simplify（remind.rs 单通道、pause.rs 单来源、main.rs 命令增删、calc.rs 删字段）+ 前端减法（删横幅 DOM/JS/CSS、徽章去分支）+ 前端小增量（调试卡 + 一处接线）。提醒触发条件一概不动。

**Tech Stack:** Tauri 2 + Rust（src-tauri），原生 HTML/CSS/JS（无框架、无构建、无 CDN），Node 脚本测试（源码断言风格）。

**设计文档：** [docs/plans/2026-09-22-reminder-notify-only-design.md](2026-09-22-reminder-notify-only-design.md)

## Global Constraints

- **不自动 git commit**，全部改动攒在工作树（用户裁决「不提交，攒着」）
- PowerShell 命令一律 `pwsh -NoProfile -Command '...'`（单引号包裹，防 `$` 展开）
- 前端无框架、无构建步骤、无 CDN
- **提醒触发条件一概不动**：久坐（活跃阈值 / 5 分钟 idle 重置 / 5 分钟冷却）、下班（工作日 / 过 pm_end / 当日有记录 / 当天一次 / 补弹）
- 手动暂停链路全保留：托盘菜单、Alt+Shift+P、`toggle_pause`、`set_manual`、`pause_monitor` 命令
- 注释与测试断言标签用中文，遵循项目现有风格
- 改前端后必须重编译 debug 版才能目检（tauri.conf.json frontendDist 编译期内嵌）

## 设计文档偏差修正（执行前必读，共 5 处）

写计划时对照源码逐一核实，发现设计文档 4 处误差 + 1 处执行注意，按下表执行（以本计划为准）：

| # | 设计文档说法 | 源码实况 | 本计划做法 |
|---|---|---|---|
| 1 | §3.3 hover_card.html 删「休息中」分支（L597 `rest_secs` 判断） | grep `rest_secs\|paused` 零匹配——L597「休息中」是休息日 `toff` 字段文案（`s.is_workday ? … : "休息中"`），与定时休息无关 | **hover_card.html 零改动** |
| 2 | §3.1 调试卡 `<section class="card" id="debugCard" hidden>`（布尔属性） | styles.css `.card { display: flex }`（L265）会覆盖 UA 的 `[hidden]` 规则，调试卡将**常显** | 改 `class="card hidden"` + app.js `classList.remove("hidden")`（项目通用 `.hidden { display:none !important }`，与其他所有 hidden 切换惯例一致） |
| 3 | §2.4「merge 白名单加 `"debug"`」 | merge_from_value（config.rs L327-337）无白名单机制——直接 insert 全字段 + serde 校验，未知字段本就放行 | config.rs 只加字段 + `#[serde(default)]`，**merge_from_value 零改动** |
| 4 | §5 test_hidden.js「重数后更新期望值」 | test_hidden.js 动态收集 hidden 元素，无固定计数断言 | **test_hidden.js 零改动**（remindBar 消失自动少计 1；debugCard 走通用 `.hidden` 规则） |
| 5 | §2.1「`Emitter`/`Manager` import 同查」 | `Manager` 被 remind.rs L156 `app.state::<AppState>()` 使用，`Emitter` 仅 emit 用 | **删 `Emitter` 和 `serde_json::json`，保留 `Manager`** |

## 测试影响盘点（先读，再动工）

| 脚本 | 动作 |
|------|------|
| `scripts/test_remind.js` | **整体重写**：旧 33 条横幅断言全删，换为「横幅已删」反向断言 + 通知单通道契约 + 命令增删 + 调试卡接线 + 保留触发契约（约 26 条） |
| `scripts/test_guard.js` | 删 14 条（休息徽章 5 + rest_secs=0 1 + 横幅 DOM 4 + capabilities 2 + pause_rest/remind_ack/ack/注册表 4——其中休息徽章按设计是 6 条口径，实为 L86/87/89/91/93 五条 eq + L100 一条），头注释更新，手动暂停段去 `rest_secs` 参数，新增 2 条（renderBadge 无 rest_secs 残留、pause_monitor 保留） |
| `scripts/test_hidden.js` | 零改动（见偏差修正 #4） |
| `scripts/test_settings.js` | 零改动（debug 只读不写，不走保存路径） |
| 其余 15 个 | 不受影响，Task 4 run_all 复核 |
| Rust 单测 | pause.rs `pause_state_machine` 删休息段；remind.rs 纯函数单测全保留 |

---

### Task 1: 测试先行（RED）

**Files:**
- Rewrite: `scripts/test_remind.js`（全文替换）
- Modify: `scripts/test_guard.js`（头注释、L84-100、L155-171）

**Interfaces:**
- Consumes: 无（纯测试）
- Produces: RED 基线——test_remind.js 约 15 FAIL / 10 PASS，test_guard.js 1 FAIL（`rest_secs` 反向断言）。Task 2/3 完成后转全绿

- [ ] **Step 1: 重写 `scripts/test_remind.js`（全文替换）**

```js
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
  ok("注册表含 test_offwork_notify", /^\s*test_offwork_notify,$/m.test(mainSrc));
  ok("注册表仍含 pause_monitor（手动暂停入口）", /^\s*pause_monitor,$/m.test(mainSrc));

  console.log("== capabilities ==");
  ok("无 allow-remind-ack", !/"allow-remind-ack"/.test(capSrc));
  ok("无 allow-pause-rest", !/"allow-pause-rest"/.test(capSrc));
  ok("有 allow-test-offwork-notify", /"allow-test-offwork-notify"/.test(capSrc));
  ok("保留 notification:default", /"notification:default"/.test(capSrc));

  console.log("== 调试卡（debug 字段驱动） ==");
  ok("调试卡 DOM 存在且默认 hidden", /id="debugCard" class="card hidden"/.test(htmlSrc));
  ok("测试下班提醒按钮存在", /id="testOffworkBtn" class="ghost">测试下班提醒</.test(htmlSrc));
  ok("load 按 cfg.debug 摘掉 hidden", /if \(cfg\.debug\)[\s\S]{0,80}\$\("debugCard"\)\.classList\.remove\("hidden"\)/.test(appSrc));
  ok("按钮接线 → test_offwork_notify", /\$\("testOffworkBtn"\)\.addEventListener\("click"[\s\S]{0,120}invoke\("test_offwork_notify"\)/.test(appSrc));
  ok("config.rs 有 debug 字段且默认 false", /pub debug: bool,/.test(configSrc) && /debug: false,/.test(configSrc));

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
```

- [ ] **Step 2: 跑 test_remind.js 确认 RED**

Run: `pwsh -NoProfile -Command 'node scripts/test_remind.js'`
Expected: FAIL 约 15 条（横幅机制、命令增删、capabilities、调试卡各段），PASS 约 10 条（触发契约段 + main.rs 内 pause_monitor）。**不允许崩异常**（脚本顶部读取的 8 个源文件都存在，不会 throw）

- [ ] **Step 3: 修改 `scripts/test_guard.js`**

3a. 头注释（L1-3）替换：

```js
// 守护·暂停徽章的回归测试：从 app.js 抽出真实 renderBadge，用桩 DOM 跑全部分支。
// 定时休息已砍（通知化改造）：徽章只剩手动暂停一种 off 态，无 rest_secs 分支。
// 用法（任意目录）：node scripts/test_guard.js
```

3b. 删 L16 的 `remindSrc` 读取行（本文件唯一使用点 L169 的 ack 断言随 Step 3c 删除）：

```js
// 原
const remindSrc = fs.readFileSync(path.join(ROOT, "src-tauri", "src", "remind.rs"), "utf8");
// 整行删除
```

3c. 「定时休息分支」整段（L84-93）删除；「手动暂停分支」段（L95-100）替换为：

```js
console.log("== 手动暂停分支 ==");
api.renderBadge(work({ paused: true }));
eq("手动暂停", badge().text, "已暂停 · 钱先冻结");
eq("暂停 className", badge().cls, "badge off");
ok("renderBadge 无 rest_secs 残留", !/rest_secs/.test(appSrc));
```

3d. 「DOM / 后端契约」段（L155-171）替换为：

```js
console.log("== DOM / 后端契约 ==");
ok("守护设置卡存在", /<h2>守护<\/h2>/.test(htmlSrc));
ok("阈值输入 1–120", /id="remind_sedentary_minutes" type="number" min="1" max="120"/.test(htmlSrc));
ok("capabilities: allow-pause-monitor", /"allow-pause-monitor"/.test(capSrc));
ok("pause_monitor 命令保留", /fn pause_monitor\(app: tauri::AppHandle\)/.test(mainSrc));
```

（删除项：横幅 DOM 4 条、allow-pause-rest / allow-remind-ack 2 条、pause_rest→rest_for(300)、remind_ack 命令、ack 重置、注册表含 pause_rest/remind_ack——共 8 条）

- [ ] **Step 4: 跑 test_guard.js 确认 RED**

Run: `pwsh -NoProfile -Command 'node scripts/test_guard.js'`
Expected: 仅 1 条 FAIL（「renderBadge 无 rest_secs 残留」，旧 app.js 还有 rest_secs），其余全 PASS

- [ ] **Step 5: 跑 test_hidden.js 确认不受测试改动影响**

Run: `pwsh -NoProfile -Command 'node scripts/test_hidden.js'`
Expected: 全 PASS（remindBar 仍存在、debugCard 未加，动态收集照常）

---

### Task 2: Rust 后端（calc → config → pause → remind → main → capabilities → cargo test）

**Files:**
- Modify: `src-tauri/src/calc.rs`（DayStatus 删字段 L33-36、构造处 L216-217）
- Modify: `src-tauri/src/config.rs`（字段区 L131 后 + Default L210）
- Modify: `src-tauri/src/pause.rs`（全文 117 行大幅缩减）
- Modify: `src-tauri/src/remind.rs`（模块文档、imports、notify、删 ack、两处调用点）
- Modify: `src-tauri/src/main.rs`（get_status L131、命令区 L407-418、注册表 L792-793）
- Modify: `src-tauri/capabilities/default.json`（L38-39）

**Interfaces:**
- Consumes: Task 1 的 RED 断言
- Produces: `notify(app, title, body)`（pub(crate)）；`test_offwork_notify` 命令；`Config.debug` 字段；`DayStatus` 无 `rest_secs`；`PauseView` 只剩 `paused`

- [ ] **Step 1: calc.rs 删 `rest_secs` 字段**

DayStatus 结构体（L33-36）：

```rust
    /// 暂停中（手动暂停）。compute 恒为 false，由 main::get_status 按全局状态覆写
    pub paused: bool,
    /// 定时休息剩余秒数（None = 非定时休息；手动暂停为 None）
    pub rest_secs: Option<i64>,
```

改为（删注释行 + 字段行，paused 的注释同步去掉「或定时休息」）：

```rust
    /// 暂停中（手动暂停）。compute 恒为 false，由 main::get_status 按全局状态覆写
    pub paused: bool,
```

构造处（L216-217）：

```rust
        paused: false,
        rest_secs: None,
```

改为：

```rust
        paused: false,
```

- [ ] **Step 2: config.rs 加 `debug` 字段**

字段区 `shortcuts_enabled`（L129-131）之后加：

```rust
    /// 全局快捷键开关（Alt+Shift+N 显隐主窗 / Alt+Shift+P 切换暂停）
    #[serde(default = "default_true")]
    pub shortcuts_enabled: bool,
    /// 调试开关：config.json 手动置 true，设置页显示「调试」卡（不放 UI 开关）
    #[serde(default)]
    pub debug: bool,
```

Default impl（L210 `shortcuts_enabled: true,` 之后）：

```rust
            shortcuts_enabled: true,
            debug: false,
            retention_days: 0,
```

**merge_from_value 零改动**（无白名单机制，见偏差修正 #3；`#[serde(default)]` 保证旧 config.json 缺字段反序列化成功）

- [ ] **Step 3: pause.rs 删定时休息链**

3a. 模块文档（L1-9）替换：

```rust
//! 手动暂停监控（v1.3.0「牛马守护」）
//!
//! 全局「暂停」状态机：托盘 / 全局快捷键 / 前端徽章共用。暂停只有一种来源：
//! 手动暂停（[`MANUAL`]）——托盘菜单或 Alt+Shift+P 切换，无限期直到再次恢复。
//!
//! 暂停语义：三类监控（键鼠 / 应用 / 音频）立即停止记账——已入账时长不回滚，
//! 未入账增量不累计（各守卫点直接丢弃，见 activity / app_usage / audio_usage
//! 的 `is_paused` 门控）。恢复瞬间不补记暂停期间的任何时长——「暂停」就是钱先冻结。
```

3b. 删 `REST_UNTIL` static（L19-20）、`now_ms()`（L22-27，唯一使用方是 rest 链）、`rest_for()`（L38-41）、`rest_remaining_secs()`（L52-56）。

3c. `set_manual`（L29-36）回归单行：

```rust
/// 切换手动暂停（托盘 / 快捷键共用）。
pub fn set_manual(v: bool) {
    MANUAL.store(v, Ordering::SeqCst);
}
```

3d. `is_paused`（L43-50）只留 MANUAL：

```rust
/// 当前是否处于暂停（手动）
pub fn is_paused() -> bool {
    MANUAL.load(Ordering::Relaxed)
}
```

3e. `PauseView` 与 `view()`（L58-70）：

```rust
/// 暂停状态视图（pause_monitor 命令返回体）
#[derive(Debug, Clone, Serialize)]
pub struct PauseView {
    pub paused: bool,
}

pub fn view() -> PauseView {
    PauseView {
        paused: is_paused(),
    }
}
```

3f. 单测 `pause_state_machine`（L72-117）删休息段（复位行 REST_UNTIL、rest_remaining_secs 断言、休息期/恢复清休息/过期/0 秒四段）：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// 状态机全序列。单用例串行：模块级 static 在 cargo test 并行下共享，
    /// 拆成多个用例会互相污染，必须按顺序走完整条状态链。
    #[test]
    fn pause_state_machine() {
        // 复位到干净起点（前序用例可能留下状态）
        set_manual(false);
        assert!(!is_paused());

        // 手动暂停 / 恢复
        set_manual(true);
        assert!(is_paused(), "手动暂停后应视为暂停");
        set_manual(false);
        assert!(!is_paused());

        // 复位，避免污染其他模块的测试
        set_manual(false);
    }
}
```

- [ ] **Step 4: remind.rs 单通道化**

4a. 模块文档（L1-16）替换：

```rust
//! 提醒（v1.3.0「牛马守护」）：久坐提醒 + 下班提醒。
//!
//! 调度：由 scheduler 每 60 秒调用一次 [`tick`]（分钟级阈值无需秒级精度）。
//! 投递：统一走 tauri-plugin-notification 系统通知（[`notify`]），
//! 主窗可见与否不影响通道（v1.3.1 起移除横幅双通道）。
//!
//! 久坐口径：
//! - 「在活跃」= 最近一次键鼠输入距今 < 5 分钟（纯挂机不键入不算连续活跃）；
//! - 暂停（手动）与锁屏期间不计时，连续起点清零；
//! - 连续活跃 ≥ 阈值分钟 → 触发；触发后清零重新起算，且 5 分钟冷却防轰炸。
//!
//! 下班口径：工作日 + 已过 `pm_end` + 当日有监控记录 + 今天未提醒过 → 触发一次。
//! 程序在过点后才启动也能补弹（today_done 初始为 false，首拍即满足）。
```

4b. imports（L22-23）：

```rust
// 原
use serde_json::json;
use tauri::{Emitter, Manager};
// 改为（json 随 payload 删，Emitter 只服务 emit；Manager 被 offwork_tick 的 app.state 使用，保留）
use tauri::Manager;
```

4c. 删 `ack()`（L90-95 整段含注释）。

4d. `sedentary_tick` 的 notify 调用（L134-139）去掉 payload：

```rust
        notify(
            app,
            "该起来活动了",
            &format!("已连续搬砖 {minutes} 分钟，起来喝口水 🐎"),
        );
```

4e. `offwork_tick` 的 notify 调用（L169-174）：

```rust
    notify(
        app,
        "到点了，下班吧牛马",
        &format!("今天已赚 ¥{:.2}，别卷了 🐎", st.earned),
    );
```

4f. `notify` 函数（L193-210）：

```rust
/// 系统通知投递（tauri-plugin-notification）。允许静默失败——提醒不该影响主流程。
pub(crate) fn notify(app: &tauri::AppHandle, title: &str, body: &str) {
    let _ = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show();
}
```

- [ ] **Step 5: main.rs 命令增删**

5a. `get_status`（L129-132）删 rest_secs 赋值行：

```rust
    // v1.3.0 守护：暂停状态随每秒状态快照广播（托盘文案 / 前端徽章 / 悬停卡片共用）
    st.paused = pause::is_paused();
    st
```

5b. 删 `pause_rest` 命令（L407-412 含注释）与 `remind_ack` 命令（L414-418 含注释）；原位置放新命令（`pause_monitor` 之后）：

```rust
/// 调试卡「测试下班提醒」：直发一条真实文案的系统通知，点一下即可验证通知通道
#[tauri::command]
fn test_offwork_notify(app: tauri::AppHandle) {
    let st = get_status(app.state::<AppState>().inner());
    remind::notify(&app, "到点了，下班吧牛马", &format!("今天已赚 ¥{:.2}，别卷了 🐎", st.earned));
}
```

（`Manager` 已在 main.rs L30 引入，无需新 use）

5c. invoke_handler 注册表（L790-793）：

```rust
            get_status_cmd,
            pause_monitor,
            test_offwork_notify,
            hide_window,
```

（删 `pause_rest,`、`remind_ack,` 两行，`test_offwork_notify` 放 `pause_monitor` 之后）

- [ ] **Step 6: capabilities/default.json**

L37-40：

```json
    "allow-pause-monitor",
    "allow-test-offwork-notify",
    "notification:default"
```

（删 `"allow-pause-rest",`、`"allow-remind-ack",` 两行）

- [ ] **Step 7: cargo test 全绿**

Run: `pwsh -NoProfile -Command 'cd src-tauri; cargo test'`
Expected: 编译零错误零警告（dead_code 会抓漏删的引用）；单测全 PASS（pause_state_machine 保留段 + remind.rs 5 个纯函数测试 + config/calc 等原有测试）。**若报 unresolved `rest_secs`/`rest_for`/`remind::ack` 等，按报错回到对应 Step 补删**

- [ ] **Step 8: 后端断言转绿（部分）**

Run: `pwsh -NoProfile -Command 'node scripts/test_guard.js'`
Expected: 全 PASS（rest_secs 反向断言在 renderBadge 未改前应仍 FAIL——app.js 前端侧未动，**此脚本要等 Task 3 Step 4 才全绿，本步只确认后端契约段无 FAIL**）

Run: `pwsh -NoProfile -Command 'node scripts/test_remind.js'`
Expected: 命令增删 / capabilities 两段转 PASS，前端段（横幅、调试卡）仍 FAIL

---

### Task 3: 前端（index.html → app.js → styles.css → 转绿）

**Files:**
- Modify: `frontend/index.html`（删横幅 L722-728、卡③文案 L480、调试卡 L589-590 之间）
- Modify: `frontend/app.js`（renderBadge L357-365、load() L92、横幅块 L2526-2575）
- Modify: `frontend/styles.css`（删 .remind-bar L853-873）
- 零改动: `frontend/hover_card.html`（见偏差修正 #1）

**Interfaces:**
- Consumes: Task 2 的后端（`cfg.debug` 字段下发、`test_offwork_notify` 命令）
- Produces: 调试卡（默认 hidden）、单通道提醒；test_remind / test_guard 全绿

- [ ] **Step 1: index.html 三处**

1a. 删提醒横幅整块（L722-728，含两行注释）：

```html
    <!-- 提醒横幅（v1.3.0 守护）：Rust 端 remind::tick 主窗可见时 emit remind-banner，
         久坐提醒带「休息 5 分钟」，下班提醒只有「知道了」；主窗隐藏时改走系统通知 -->
    <div id="remindBar" class="remind-bar hidden">
      <span id="remindMsg"></span>
      <button id="remindOk" class="ghost">知道了</button>
      <button id="remindRest" class="primary hidden">休息 5 分钟</button>
    </div>
```

（删后 L721 `</div>` 直接接 L729 `<div id="toast" ...>`）

1b. 卡③ 守护 `<details>` 文案（L480）：

```html
      <p>提醒通过系统通知发送；快捷键与其他软件冲突时可关闭此开关</p>
```

1c. 调试卡插在卡⑥ `</section>`（L589）与 `viewSettings` 闭合 `</div>`（L590）之间：

```html
  </section>

  <!-- 调试卡：config.json 置 debug: true 才显示（load 时按配置摘掉 hidden） -->
  <section class="card hidden" id="debugCard">
    <h2>调试</h2>
    <button id="testOffworkBtn" class="ghost">测试下班提醒</button>
  </section>
</div>
```

- [ ] **Step 2: app.js 三处**

2a. `renderBadge`（L357-365）删 rest_secs 分支，注释同步：

```js
// 状态徽章：主页品牌行右侧的动态状态（搬砖中 / 已下班 / 今天休息）。
function renderBadge(s) {
  const badge = $("statusBadge");
  if (s.paused) {
    badge.textContent = "已暂停 · 钱先冻结";
    badge.className = "badge off";
  } else if (!s.is_workday) {
```

（原 L358-359 的「定时休息要先于手动暂停判断」注释与 L362-365 的 rest_secs 分支整段删除，`else if (s.paused)` 提为 `if (s.paused)`）

2b. `load()` 末尾（L92 `setBillStyleUI(...)` 与 L94 `lastSaved = ...` 之间）加：

```js
    setBillStyleUI(cfg.bill_style || "receipt");
    loadStorageInfo();
    // 调试卡：config.json 手动置 debug: true 才显示（不放 UI 开关）
    if (cfg.debug) $("debugCard").classList.remove("hidden");
    // 初始快照：与 readCfg() 字段顺序一致，用于失焦保存时判断是否有变化
    lastSaved = JSON.stringify(readCfg());
```

2c. 删横幅整块（L2526-2571：段注释、remindTimer、showRemindBar、hideRemindBar、watchRemindBanner、remindOk/remindRest 两段按钮接线），并把 L2573-2575 的挂载三连改为两处 + 按钮接线：

```js
// 调试卡「测试下班提醒」：直发一条真实文案的系统通知，点一下即可验证通知通道
// （调试卡由 config.json debug 字段控制显隐，见 load()）
$("testOffworkBtn").addEventListener("click", () => {
  invoke("test_offwork_notify").catch(() => {});
});

boot();
watchVisibility();
```

（删 `watchRemindBanner();` 挂载行）

- [ ] **Step 3: styles.css 删 .remind-bar**

删 L853-873（注释行 + `.remind-bar { … }` + `.remind-bar span { … }`），删后 L851 `}` 直接接 L875 `/* ====== 自定义确认弹窗 ====== */`。

- [ ] **Step 4: 测试转绿**

Run: `pwsh -NoProfile -Command 'node scripts/test_remind.js; node scripts/test_guard.js; node scripts/test_hidden.js'`
Expected: 三个脚本全 PASS（test_hidden 动态少计 remindBar 后照常通过）

---

### Task 4: 收尾（run_all + build + CHANGELOG + 手测清单）

**Files:**
- Modify: `CHANGELOG.md`（[未发布] 区三条改动）
- Test: `scripts/run_all.js`

**Interfaces:**
- Consumes: Task 1-3 全部产出
- Produces: 全绿回归 + 与源码一致的 debug 构建 + 可手测交付

- [ ] **Step 1: 全量回归**

Run: `pwsh -NoProfile -Command 'node scripts/run_all.js'`
Expected: 19 个脚本全部 0 failed（总断言数因 test_remind 重写会少于原 701，记录实际值）

- [ ] **Step 2: build 冒烟**

Run: `pwsh -NoProfile -Command '& .\build.bat debug'`（仓库根目录）
Expected: 构建成功，`bin\debug\niuma-timer.exe` 更新（前端内嵌进 exe，目检必须用新构建）

- [ ] **Step 3: CHANGELOG [未发布] 区**

3a. 改写「久坐提醒」条目（L11），删双通道/横幅/休息/知道了描述，句尾改：

```markdown
- **久坐提醒**——连续活跃超过阈值（设置页可调，默认 50 分钟，钳制 1–120）时提醒起来活动。判定在 `remind.rs` 分钟级 tick：连续活跃按「无 5 分钟以上键鼠空闲」计，起身倒水即重置起算点；提醒后进入 5 分钟冷却，不连发轰炸。提醒统一走系统通知
```

3b. 改写「下班提醒」条目（L13）：

```markdown
- **下班提醒**——工作日 + 已过下班时间（`pm_end`）+ 当日有监控记录时，系统通知「到点了，下班吧牛马 · 今天已赚 ¥X」，把当天实时进账直接递到眼前。当天只提醒一次；程序在过点后才启动也能补弹（首拍即满足全部条件，无需特殊分支）
```

3c. `### 新增` 区追加调试模式条（设置页重组条目之后）：

```markdown
- **调试模式**——`config.json` 手动置 `"debug": true` 后，设置页最底出现「调试」卡，内含「测试下班提醒」按钮：直发一条真实文案的系统通知，一键验证通知通道是否可用（不放 UI 开关，配置缺字段默认 false）
```

3d. `### 新增` 区之前插入移除区：

```markdown
### 移除

- **提醒横幅与「休息 5 分钟」**——久坐 / 下班提醒不再弹应用内横幅，统一走系统通知（主窗可见与否不影响通道）；「知道了」与「休息 5 分钟」按钮随之移除：触发瞬间后端已重置起算点并进入冷却，ack 本就冗余，想歇只能托盘手动暂停（手动暂停链路不变）。同步删除 `pause_rest` / `remind_ack` 命令、`rest_secs` 状态字段与徽章「休息中」倒计时
```

- [ ] **Step 4: 向用户输出手测清单**（文字输出，不改代码）

1. `config.json` 置 `"debug": true` → 重启 → 设置页最底出现「调试」卡；置回 false → 消失
2. 点「测试下班提醒」→ 系统通知「到点了，下班吧牛马 / 今天已赚 ¥X，别卷了 🐎」；**不弹则查 Windows 通知设置/勿扰模式**——这就是本次加按钮的目的
3. 久坐提醒：阈值改 1 分钟，持续活跃 → 收到系统通知（主窗可见也**不再**弹横幅）
4. 下班提醒：`pm_end` 改到 2 分钟后，**无需藏窗口** → 收到系统通知
5. 托盘「暂停监控」→ 徽章「已暂停 · 钱先冻结」；「恢复监控」→ 徽章回正常；「休息中」徽章不再出现
6. 设置页六卡结构与上轮交付一致，无横幅残留、无布局破相

---

## 自查记录

- **Spec 覆盖**：设计 §2 后端 → Task 2（remind/pause/main/calc/config/capabilities 六处全覆盖，含偏差修正 #3/#5）；§3 前端 → Task 3（index/app/styles，hover_card 按偏差修正 #1 零改动，调试卡按偏差修正 #2 改 class 方案）；§4 行为变更 → Task 4 Step 4 手测清单；§5 测试影响 → 盘点表 + Task 1/3/4（test_hidden 按偏差修正 #4 零改动）；§7 CHANGELOG → Task 4 Step 3
- **引用面闭环**（全仓库 grep 核实）：`rest_for`/`rest_remaining_secs`/`REST_UNTIL` 只存在于 pause.rs + main.rs L131/409-411；`remind::ack` 只在 main.rs L417；`PauseView` 只被 main.rs 的 pause_monitor（保留）/pause_rest（删）使用；前端 `rest_secs` 只在 app.js L362-364；scheduler.rs / tray.rs / hover_card.html 无任何 rest / banner 引用
- **占位符**：无 TBD/TODO；所有修改步骤给出完整旧代码与目标代码
- **类型/命名一致**：`test_offwork_notify` 在 main.rs 定义、注册表、capabilities（`allow-test-offwork-notify`）、app.js 接线四处拼写一致；`debugCard`/`testOffworkBtn` 在 index.html 与 app.js 一致；`cfg.debug` 与 config.rs `pub debug: bool` 一致
- **TDD 顺序**：Task 1 RED（约 16 FAIL）→ Task 2 转绿后端段 → Task 3 转绿全部 → Task 4 全量兜底
