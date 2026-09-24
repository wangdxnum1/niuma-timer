# 调试模式彩蛋（进程级，不落盘）— 设计

- 日期：2026-09-22
- 状态：已与用户对齐，待实施
- 前置：主线 B（提醒纯系统通知化 + 调试模式）已交付——调试卡现由 config.json 的 `debug` 字段驱动。本设计将其改为进程级彩蛋开关。

## 1. 需求与裁决

用户裁决（2026-09-22）：
1. 设置按钮 **2 秒内连点 5 次** = 调试模式开关（双向 toggle）
2. **不落盘**：只对当前进程有效，重启后默认关闭，需重新连点开启
3. **去掉配置文件逻辑**：删除 config.rs 的 `debug` 字段，唯一入口就是连点彩蛋
4. 提示用 **toast 气泡**（「调试模式已开启 / 已关闭」）

## 2. 交互

- **滑动窗口**：每次点击重置 2 秒计时；任意相邻两次点击间隔 < 2s 的 5 连击触发
- 第 5 击 → 翻转 debug 态并清零计数（下次再连 5 击翻回）
- 开：调试卡立即显示（设置页最底）+ toast「调试模式已开启」
- 关：调试卡立即隐藏 + toast「调试模式已关闭」
- 连点期间每次点击照常执行导航（showView("viewSettings")），彩蛋与导航互不干扰
- 窗口内不足 5 次 → 计时器归零，无任何效果（只是正常导航）

## 3. 实现面

### 3.1 frontend/app.js（核心改动）

- 删 load() 中 L93-94（`// 调试卡：config.json 手动置 debug: true…` 注释 + `if (cfg.debug) $("debugCard").classList.remove("hidden");`）
- 新增模块级状态与 toggleDebug()；挂接点在 `.rail-item` 统一 click 循环（app.js:2360-2364），对 `data-nav="viewSettings"` 的按钮计数：

```js
// 调试模式彩蛋：2 秒内连点设置按钮 5 次开关（进程级，不落盘，重启失效）
let debugOn = false;
let eggClicks = 0;
let eggTimer = 0;

function toggleDebug() {
  debugOn = !debugOn;
  $("debugCard").classList.toggle("hidden", !debugOn);
  showToast(debugOn ? "调试模式已开启" : "调试模式已关闭", "ok");
}
```

```js
document.querySelectorAll(".rail-item").forEach((btn) => {
  btn.addEventListener("click", () => {
    // 调试彩蛋：设置按钮 2 秒内连点 5 次翻转调试模式（滑动窗口，每击重置计时）
    if (btn.dataset.nav === "viewSettings") {
      eggClicks++;
      clearTimeout(eggTimer);
      eggTimer = setTimeout(() => (eggClicks = 0), 2000);
      if (eggClicks >= 5) {
        eggClicks = 0;
        toggleDebug();
      }
    }
    showView(btn.dataset.nav === "detail" ? lastDetailView : btn.dataset.nav);
  });
});
```

- `toggleDebug` 为顶层函数声明（有提升），放接线代码附近即可；`showToast`（app.js:289）同为顶层声明，可直接调用

### 3.2 src-tauri/src/config.rs（回退）

- 删 L132-134：`/// 调试开关：…` 注释 + `#[serde(default)]` + `pub debug: bool,`
- 删 Default impl 中 `debug: false,`（L214）
- Config 未启用 `deny_unknown_fields`，旧 config.json 残留 `"debug": true` 会被 serde 静默忽略，兼容无忧

### 3.3 frontend/index.html（注释口径）

- L591 注释改：「调试卡：2 秒内连点设置按钮 5 次开关（进程级，重启失效）」
- DOM 结构与初始 `class="card hidden"` 不变（初始必隐藏，由彩蛋摘除）

### 3.4 后端其余零改动

- `test_offwork_notify` 命令 + `allow-test-offwork-notify` 权限原样保留——命令本来就一直注册，调试态只管前端 UI 暴露

## 4. 测试影响

### scripts/test_remind.js「调试卡」段改写（5 → 7 条）

- 保留：L62 调试卡 DOM 默认 hidden、L63 测试按钮存在、L65 按钮接线 → test_offwork_notify
- 删除：L64「load 按 cfg.debug 摘掉 hidden」、L66「config.rs 有 debug 字段且默认 false」
- 新增（4 条）：
  - 彩蛋计数器存在：app.js 含 `viewSettings` 判定 + `eggClicks++` + `setTimeout(..., 2000)`
  - 第 5 击触发：`eggClicks >= 5` → `toggleDebug()`
  - toggleDebug 契约：函数体内同时含 `classList.toggle("hidden"` 与 `showToast(`
  - toast 文案：含「调试模式已开启」与「调试模式已关闭」
- 段名 L61 改：`== 调试卡（彩蛋驱动） ==`

### 其余测试

- test_guard.js / test_settings.js / test_hidden.js：无 debug 相关断言，不动
- 完成后跑 `node scripts/run_all.js` 全量回归兜底；cargo test 兜底 config.rs 回退

## 5. CHANGELOG [未发布]

- 「调试模式」条目改口径：由「config.json 置 debug: true」改为「2 秒内连点设置按钮 5 次开关调试模式（进程级，不落盘，重启失效）」
- 其余条目不动

## 6. 错误处理与边界

- toggle 纯前端（无 invoke），无失败路径；toast 复用 showToast("ok")
- 第 5 击那次的导航照常执行（最后一次点击仍进设置页）
- 计数器单定时器复用（clearTimeout 先行），无泄漏
- 与「失焦自动保存」无交互：readCfg() 不含 debug，lastSaved 快照不受影响

## 7. 非目标（YAGNI）

- 不做 debug 态的托盘/徽章提示（调试卡出现本身即提示）
- 不做彩蛋进度提示（如「还差 2 次」）——保持隐蔽
- 不做设置页显式开关
