# 调试模式彩蛋（进程级，不落盘）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 设置按钮 2 秒内连点 5 次开关调试模式（进程级 toggle，不落盘），并删除 config.rs 的 `debug` 字段，调试卡改为纯前端彩蛋驱动。

**Architecture:** toggle 纯前端实现（内存布尔 + debugCard 显隐 + showToast），零新 Tauri 命令；config.rs 回退两行删除（serde 无 deny_unknown_fields，旧配置残留键无害）；测试先行 RED → 前端转绿 → 收尾兜底。

**Tech Stack:** Tauri 2 + Rust（src-tauri）+ 原生 JS 无构建前端（frontend/）+ Node 零依赖契约测试（scripts/）

**设计文档:** `docs/plans/2026-09-22-debug-easter-egg-design.md`

## Global Constraints

- **无 git commit**（用户裁决「不提交，攒着」）——计划不含任何 commit 步骤
- PowerShell 一律 `pwsh -NoProfile -Command '...'` 单引号包裹
- 注释与断言标签一律中文
- 前端无框架、无构建、无 CDN，改 frontend/ 后必须重编译才能目检（frontendDist 编译期内嵌进 exe）
- build.rs 缓存戳副作用：构建会自动改写 index.html 两处 `?v=` 与 app.js 的 `FE_VER`/boot 注释——实施后这些行变化属预期，非手工改动
- toast 文案精确值：「调试模式已开启」/「调试模式已关闭」（type 用 "ok"）
- `test_offwork_notify` 命令与 `allow-test-offwork-notify` 权限**不动**（调试态只管前端 UI 暴露，后端命令一直注册）

---

### Task 1: 测试先行（RED）+ config.rs 回退

**Files:**
- Modify: `scripts/test_remind.js:61-66`（调试卡段改写）
- Modify: `src-tauri/src/config.rs:132-134,214`（删 debug 字段）
- Test: `scripts/test_remind.js` + `cargo test`

**Interfaces:**
- Consumes: 现有 test_remind.js 六段结构（本任务只动「调试卡」段）
- Produces: 调试段 7 条断言（其中 4 条彩蛋契约断言在 Task 2 前保持 FAIL）；config.rs 无 debug 字段（Task 2 依赖此终态做 cargo test 兜底）

- [ ] **Step 1: 改写 test_remind.js「调试卡」段（RED）**

精确替换 `scripts/test_remind.js` L61-66（旧代码）：

```js
  console.log("== 调试卡（debug 字段驱动） ==");
  ok("调试卡 DOM 存在且默认 hidden", /id="debugCard" class="card hidden"/.test(htmlSrc));
  ok("测试下班提醒按钮存在", /id="testOffworkBtn" class="ghost">测试下班提醒</.test(htmlSrc));
  ok("load 按 cfg.debug 摘掉 hidden", /if \(cfg\.debug\)[\s\S]{0,80}\$\("debugCard"\)\.classList\.remove\("hidden"\)/.test(appSrc));
  ok("按钮接线 → test_offwork_notify", /\$\("testOffworkBtn"\)\.addEventListener\("click"[\s\S]{0,120}invoke\("test_offwork_notify"\)/.test(appSrc));
  ok("config.rs 有 debug 字段且默认 false", /pub debug: bool,/.test(configSrc) && /debug: false,/.test(configSrc));
```

替换为（新代码，7 条）：

```js
  console.log("== 调试卡（彩蛋驱动） ==");
  ok("调试卡 DOM 存在且默认 hidden", /id="debugCard" class="card hidden"/.test(htmlSrc));
  ok("测试下班提醒按钮存在", /id="testOffworkBtn" class="ghost">测试下班提醒</.test(htmlSrc));
  ok("按钮接线 → test_offwork_notify", /\$\("testOffworkBtn"\)\.addEventListener\("click"[\s\S]{0,120}invoke\("test_offwork_notify"\)/.test(appSrc));
  ok(
    "彩蛋计数器：viewSettings 判定 + eggClicks 自增 + 2s 窗口",
    /nav === "viewSettings"/.test(appSrc) &&
      /eggClicks\+\+/.test(appSrc) &&
      /setTimeout\(\(\) => \(eggClicks = 0\), 2000\)/.test(appSrc)
  );
  ok("第 5 击触发 toggleDebug", /eggClicks >= 5[\s\S]{0,60}toggleDebug\(\)/.test(appSrc));
  ok(
    "toggleDebug：显隐 + toast",
    /function toggleDebug\(\)[\s\S]{0,200}classList\.toggle\("hidden"[\s\S]{0,120}showToast\(/.test(appSrc)
  );
  ok("toast 文案：已开启/已关闭", /调试模式已开启/.test(appSrc) && /调试模式已关闭/.test(appSrc));
```

- [ ] **Step 2: 跑测试验证 RED 基线**

Run: `pwsh -NoProfile -Command 'node scripts/test_remind.js'`
Expected: **28 PASS / 4 FAIL**——4 条新增彩蛋断言 FAIL（彩蛋未实现），其余全部 PASS（总断言 30 − 2 删 + 4 增 = 32；config 相关两条已删，config.rs 此时尚未回退也无碍）

- [ ] **Step 3: config.rs 删 debug 字段**

精确替换 `src-tauri/src/config.rs` L132-134（旧代码）：

```rust
    /// 调试开关：config.json 手动置 true，设置页显示「调试」卡（不放 UI 开关）
    #[serde(default)]
    pub debug: bool,
```

替换为：**直接删除这三行**（`shortcuts_enabled` 字段后直接接空行与 `// ---- 数据保留 ----`）。

精确替换 Default impl 中 L213-215（旧代码）：

```rust
            shortcuts_enabled: true,
            debug: false,
            retention_days: 0,
```

替换为（新代码）：

```rust
            shortcuts_enabled: true,
            retention_days: 0,
```

兼容性依据：Config 未启用 `deny_unknown_fields`，旧 config.json 残留 `"debug": true` 会被 serde 静默忽略。

- [ ] **Step 4: cargo test 兜底（后端回退无破坏）**

Run（cwd = `src-tauri`）: `pwsh -NoProfile -Command 'cargo test'`
Expected: 全部通过、0 failed、无编译 warning（main.rs 等无 `cfg.debug` 引用，纯字段删除）

- [ ] **Step 5: 复跑前端契约测试确认无副作用**

Run: `pwsh -NoProfile -Command 'node scripts/test_remind.js'`
Expected: 仍 **28 PASS / 4 FAIL**（config 相关断言已删，config.rs 回退不影响 JS 测试；4 条 FAIL 归 Task 2）

---

### Task 2: 前端彩蛋转绿 + 收尾

**Files:**
- Modify: `frontend/app.js:93-94`（删 load() 配置判断）
- Modify: `frontend/app.js:2358-2364`（rail-item 循环 + toggleDebug）
- Modify: `frontend/index.html:591`（注释口径）
- Modify: `CHANGELOG.md:23`（调试条目改口径）
- Test: `scripts/test_remind.js` + `scripts/run_all.js` + build 冒烟

**Interfaces:**
- Consumes: Task 1 的 4 条 RED 断言（本任务转绿）；现有 `showToast(msg, type)`（app.js:289，顶层函数声明可直接调用）
- Produces: `toggleDebug()` 顶层函数（进程级 debug 开关唯一入口）；`debugOn` / `eggClicks` / `eggTimer` 模块级状态

- [ ] **Step 1: 删 load() 中 config.debug 判断**

精确替换 `frontend/app.js` L93-94（旧代码）：

```js
    // 调试卡：config.json 手动置 debug: true 才显示（不放 UI 开关）
    if (cfg.debug) $("debugCard").classList.remove("hidden");
```

替换为：**直接删除这两行**（`loadStorageInfo();` 后直接接 `// 初始快照…` 注释行）。

- [ ] **Step 2: 实现彩蛋（toggleDebug + rail-item 循环改造）**

精确替换 `frontend/app.js` L2358-2364（旧代码）：

```js
// 侧边导航栏 + 明细翻页器：任意视图直达（取代原「‹ 返回」的网页式导航）。
// 自动保存（离开设置页）与历史日期复位（回主页）仍由 showView 统一处理
document.querySelectorAll(".rail-item").forEach((btn) => {
  btn.addEventListener("click", () =>
    showView(btn.dataset.nav === "detail" ? lastDetailView : btn.dataset.nav)
  );
});
```

替换为（新代码）：

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

// 侧边导航栏 + 明细翻页器：任意视图直达（取代原「‹ 返回」的网页式导航）。
// 自动保存（离开设置页）与历史日期复位（回主页）仍由 showView 统一处理
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

- [ ] **Step 3: index.html 调试卡注释改口径**

精确替换 `frontend/index.html` L591（旧代码）：

```html
  <!-- 调试卡：config.json 置 debug: true 才显示（load 时按配置摘掉 hidden） -->
```

替换为（新代码）：

```html
  <!-- 调试卡：2 秒内连点设置按钮 5 次开关（进程级，重启失效，初始必隐藏） -->
```

DOM 结构与 `class="card hidden"` 不变。

- [ ] **Step 4: 跑测试验证转绿**

Run: `pwsh -NoProfile -Command 'node scripts/test_remind.js'`
Expected: **32 PASS / 0 FAIL**

- [ ] **Step 5: 全量回归**

Run: `pwsh -NoProfile -Command 'node scripts/run_all.js'`
Expected: 19 个测试脚本全部 0 failed（总断言数约 684 − 2 + 4 = 686，以实际输出为准）

- [ ] **Step 6: CHANGELOG 调试条目改口径**

精确替换 `CHANGELOG.md` L23（旧代码）：

```markdown
- **调试模式**——`config.json` 手动置 `"debug": true` 后，设置页最底出现「调试」卡，内含「测试下班提醒」按钮：直发一条真实文案的系统通知，一键验证通知通道是否可用（不放 UI 开关，配置缺字段默认 false）
```

替换为（新代码）：

```markdown
- **调试模式（彩蛋）**——2 秒内连点侧栏「设置」按钮 5 次，设置页最底出现「调试」卡（再连点 5 次隐藏），内含「测试下班提醒」按钮：直发一条真实文案的系统通知，一键验证通知通道是否可用。进程级开关不落盘，重启后默认关闭，不占任何 UI 开关与配置字段
```

- [ ] **Step 7: build 冒烟**

Run（仓库根目录）: `pwsh -NoProfile -Command '& .\build.bat debug'`
Expected: 构建成功，`bin\debug\niuma-timer.exe` 时间戳更新（前端已内嵌，目检必须用新构建；build.rs 缓存戳改写 index.html/app.js 属预期副作用）

- [ ] **Step 8: 向用户输出手测清单（文字输出，不改代码）**

1. 连点侧栏「设置」按钮 5 次（相邻间隔 < 2 秒）→ toast「调试模式已开启」+ 设置页最底出现「调试」卡
2. 再连点 5 次 → toast「调试模式已关闭」+ 「调试」卡隐藏
3. 重启应用 → 「调试」卡默认隐藏（进程级，不落盘）
4. 开启调试后点「测试下班提醒」→ 系统通知「到点了，下班吧牛马 / 今天已赚 ¥X，别卷了 🐎」；不弹则查 Windows 通知设置/勿扰模式
5. 旧 config.json 若已手动写入 `"debug": true`，启动正常即验证通过（serde 静默忽略残留键）
6. 单击 / 双击设置按钮 → 仅正常导航，无 toast、无卡片闪现

---

## 自查记录

- **Spec 覆盖**：设计 §2 交互 → Task 2 Step 2（滑动窗口/双向/导航兼容）；§3.1 → Task 2 Step 1/2；§3.2 → Task 1 Step 3；§3.3 → Task 2 Step 3；§3.4 零改动（Global Constraints 声明）；§4 → Task 1 Step 1（7 条断言明细）+ Task 2 Step 4/5；§5 → Task 2 Step 6；§6 边界 → 实现代码体现（单定时器复用、lastSaved 不受影响因 readCfg 无 debug）；§7 非目标不做
- **占位符**：无 TBD/TODO；所有修改步骤给出完整旧代码与目标代码
- **命名/正则一致**：`toggleDebug` / `debugOn` / `eggClicks` / `eggTimer` 在实现代码与测试断言正则间逐字一致（`eggClicks\+\+`、`eggClicks >= 5[\s\S]{0,60}toggleDebug\(\)`、`function toggleDebug\(\)[\s\S]{0,200}classList\.toggle\("hidden"`）；toast 文案三元表达式同时满足「已开启/已关闭」两条断言
- **TDD 顺序**：Task 1 RED（4F）→ Task 2 前端转绿（32P/0F）→ run_all 兜底 → build 冒烟
- **算术核对**：test_remind 总断言 30 − 2 + 4 = 32；RED 期 28P/4F
