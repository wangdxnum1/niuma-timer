# 设计：提醒纯系统通知化 + 调试模式

日期：2026-09-22
状态：已与用户逐项确认，待批准执行
关联：docs/plans/2026-09-20-settings-page-redesign-*（六卡结构不动，本设计只改卡③ hint 措辞、删横幅、加调试卡）

## 0. 背景与目标

下班提醒手测时系统通知通道未触发，根因排查受制于双通道机制（主窗可见走横幅、隐藏走系统通知）——测试要卡 60 秒拍点藏窗口，且横幅弹在已隐藏的主窗里时用户什么都看不到，「当天已提醒」标记却已置位。

用户裁决：

1. **所有提醒（久坐 + 下班）只走系统通知**，整套横幅机制移除
2. **「休息 5 分钟」定时休息功能砍掉**（提醒保留），相关死代码全清
3. config.json 加 `debug` 字段，置 true 时设置页显示「调试」卡，先放一个「测试下班提醒」按钮直发系统通知

## 1. 决策表

| 决策点 | 结论 |
|---|---|
| 横幅去留 | 久坐 + 下班全部只走系统通知，横幅 UI/事件/样式全删 |
| 「休息 5 分钟」 | 砍掉：`pause_rest` 命令、`pause.rs` 定时休息链路、徽章/悬停卡「休息中」分支 |
| 「知道了」ack | 删：触发瞬间后端已重置起算点 + 5 分钟冷却，横幅回调本就冗余 |
| 触发条件 | **一概不动**：久坐（活跃阈值/5 分钟 idle 重置/冷却）、下班（工作日/过点/有记录/当天一次/补弹） |
| debug 字段 | config.json 手动改，设置页**不放**开关；serde 缺省 false |
| 调试卡位置 | 设置页最底（系统与数据之后），默认 hidden |
| 测试按钮行为 | 复用 `notify()` 发真实下班文案，earned 取当前实时进账 |
| Rust 侧门禁 | 不做：按钮藏在 hidden 卡后已够，命令本身无害 |
| 手动暂停 | 不动：托盘「暂停监控/恢复监控」、Alt+Shift+P、`toggle_pause`、`set_manual` 链路全保留 |
| `pause_monitor` 命令 | **不动但披露**：托盘/快捷键都直调 `toggle_pause`，该命令现无任何调用方（历史遗留死命令），本设计不扩权处理，记入账本待后续裁决 |

## 2. 后端改动

### 2.1 remind.rs

- `notify(app, payload, title, body)` → `notify(app, title, body)`：删 `is_visible` 判断、`emit("remind-banner")` 分支与 payload 参数；只发系统通知。改为 `pub(crate)`（新命令要调）
- 调用点同步：`sedentary_tick` / `offwork_tick` 不再组 `json!` payload（`serde_json` import 若无他用一并删；`Emitter`/`Manager` import 同查）
- 删 `ack()`（`remind_ack` 的后端实现）
- 模块文档（L5-13）同步：删「双通道投递」「横幅按钮走 rest_for」描述，改为「统一系统通知」

### 2.2 pause.rs

- 删 `REST_UNTIL` 静态、`rest_for()`、`rest_remaining_secs()`
- `set_manual()`：删 `if !v` 时清 `REST_UNTIL` 的分支，回归单行 store；注释同步
- `is_paused()`：删 `REST_UNTIL` 到期检查，只留 `MANUAL`
- `PauseView`：只剩 `paused: bool`
- 单测 `pause_state_machine`（一条串行链）：删其中休息段（复位行的 REST_UNTIL、休息期/恢复清休息/过期/0 秒四段），保留手动暂停/恢复段
- 模块文档：删「定时休息（REST_UNTIL）」来源句——暂停只剩手动一种来源（托盘/快捷键切换）

### 2.3 main.rs / calc.rs + capabilities/default.json

- `calc::DayStatus` 删 `rest_secs` 字段；main.rs L131 `st.rest_secs = pause::rest_remaining_secs();` 删除——`get_status` 载荷不再含 `rest_secs`（前端徽章/悬停卡的 `s.rest_secs` 分支随 §3 一并删）
- 删命令 `pause_rest`（L409-410）、`remind_ack`（L416）及注册（L792-793）
- 新命令：

```rust
// 调试卡「测试下班提醒」：直发一条真实文案的系统通知，点一下即可验证通知通道
#[tauri::command]
fn test_offwork_notify(app: tauri::AppHandle) {
    let st = crate::get_status(app.state::<crate::AppState>().inner());
    remind::notify(&app, "到点了，下班吧牛马", &format!("今天已赚 ¥{:.2}，别卷了 🐎", st.earned));
}
```

- capabilities：删 `allow-pause-rest`、`allow-remind-ack`；加 `allow-test-offwork-notify`；保留 `allow-pause-monitor`、`notification:default`

### 2.4 config.rs

- 新增字段（守护区之后）：

```rust
/// 调试开关：config.json 手动置 true，设置页显示「调试」卡（不放 UI 开关）
#[serde(default)]
pub debug: bool,
```

- `Default` impl 加 `debug: false`；merge 白名单加 `"debug"`

## 3. 前端改动

### 3.1 index.html

- 删提醒横幅整块（L722-728：`remindBar`/`remindMsg`/`remindOk`/`remindRest` 及注释）
- 设置页最底（系统与数据卡之后、`viewSettings` 闭合前）加：

```html
<!-- 调试卡：config.json 置 debug: true 才显示（load 时按配置摘掉 hidden） -->
<section class="card" id="debugCard" hidden>
  <h2>调试</h2>
  <button id="testOffworkBtn" class="ghost">测试下班提醒</button>
</section>
```

- 卡③ 守护 `<details>` 文案：「主窗隐藏时提醒改为系统通知；快捷键与其他软件冲突时可关闭此开关」→「提醒通过系统通知发送；快捷键与其他软件冲突时可关闭此开关」

### 3.2 app.js

- 删横幅整块（约 L2527-2575）：`showRemindBar` / `hideRemindBar` / `remind-banner` 监听挂载 / `remindOk`、`remindRest` 按钮接线 / 90 秒自动收起定时器
- `renderBadge`（L358-365）：删 `s.rest_secs`「休息中 · N 分钟后恢复」分支与对应注释，只留 `paused`「已暂停 · 钱先冻结」
- `load()` 末尾加：`if (cfg.debug) $("debugCard").hidden = false;`
- 按钮接线：`$("testOffworkBtn").addEventListener("click", () => invoke("test_offwork_notify"));`

### 3.3 hover_card.html

- 删「休息中」分支（L597 一带，`rest_secs` 判断），只留「已暂停」/正常态

### 3.4 styles.css

- 删 `.remind-bar` 相关样式块

## 4. 行为变更清单（用户可感知）

1. 久坐/下班提醒一律系统通知，主窗可见时**不再**弹横幅
2. 「休息 5 分钟」定时休息不存在了：久坐通知后想歇只能托盘手动暂停，恢复也要手动
3. `config.json` 置 `"debug": true` → 设置页最底出现调试卡，可一键测试下班提醒通知

## 5. 测试影响

| 脚本 | 影响 |
|---|---|
| test_remind.js | 重写：横幅渲染/监听/按钮接线断言删光；保留触发契约（冷却、idle 重置、补弹、config 默认值、`notification:default`）；新增：remind.rs 不再 emit remind-banner、`remind_ack`/`pause_rest` 命令不存在、capabilities 无 allow-remind-ack/allow-pause-rest、有 allow-test-offwork-notify、调试卡存在且默认 hidden、按钮 invoke 接线 |
| test_guard.js | 删「休息中」徽章 4 条（L86/89/91/93）、`rest_secs=0` 1 条（L100）、`pause_rest→rest_for(300)` 1 条（L165）及 L3 相关注释；保留「手动暂停」「已暂停 · 钱先冻结」断言 |
| test_hidden.js | 隐藏元素计数：−remindBar、+debugCard（重数后更新期望值） |
| test_settings.js | 无影响：debug 只读不写（UI 不放开关、不走保存路径），load 侧接线由 test_remind.js 的调试卡断言覆盖 |
| 其余 15 个 | 不受影响（run_all 复核） |
| Rust 单测 | pause.rs：`pause_state_machine` 删其中休息段（保留手动暂停段）；remind.rs 纯函数测试全保留 |

## 6. 验收与手测清单

1. `config.json` 置 `"debug": true` → 重启 → 设置页最底出现「调试」卡；置回 false → 消失
2. 点「测试下班提醒」→ 系统通知「到点了，下班吧牛马 / 今天已赚 ¥X，别卷了 🐎」；**不弹则查 Windows 通知设置/勿扰模式**——这就是本次加按钮的目的
3. 久坐提醒：阈值改 1 分钟，持续活跃 → 收到系统通知（不弹横幅）
4. 下班提醒：pm_end 改 2 分钟后，**无需藏窗口** → 收到系统通知
5. 托盘「暂停监控」→ 徽章「已暂停 · 钱先冻结」；「恢复监控」→ 徽章回正常
6. `node scripts/run_all.js` 19 脚本全绿；`build.bat debug` 冒烟

## 7. CHANGELOG（[未发布] 区）

- 久坐提醒条目：删「双通道/横幅/休息 5 分钟/知道了」描述，改为系统通知 + 保留冷却口径
- 下班提醒条目：删「主窗隐藏时降级为系统通知」，改为始终系统通知
- 新增一条：调试模式（`debug` 字段 + 调试卡 + 测试下班提醒按钮）
