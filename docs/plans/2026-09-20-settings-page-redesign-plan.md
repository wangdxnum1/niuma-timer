# 设置页重设计（信息架构重组）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 按已批准设计文档重组设置页为 6 张语义化卡片，长提示收进 `<details>` 折叠，久坐阈值下限放宽到 1 分钟。

**Architecture:** 纯前端重构——index.html 里 `viewSettings` 的 DOM 节点重新归位到 6 张新卡（所有元素 ID 不变），styles.css 新增 3 个小类，app.js 仅改一处钳制数值。Rust 后端零改动。

**Tech Stack:** 原生 HTML/CSS/JS（无框架、无构建、无 CDN），Node 脚本测试（正则/源码断言风格）。

**设计文档：** [docs/plans/2026-09-20-settings-page-redesign-design.md](2026-09-20-settings-page-redesign-design.md)

## Global Constraints

- 前端无框架、无构建步骤、无 CDN
- **所有设置项元素 ID 一律不变**（多个测试脚本做全量 ID 存在性兜底，见 §测试影响盘点）
- 保留测试断言的关键文本原样：`<h2>守护</h2>`、`Alt+Shift+N 显隐主窗`、`Alt+Shift+P 暂停/恢复监控`
- PowerShell 命令一律 `pwsh -NoProfile -Command '...'`（单引号包裹，防 `$` 展开）
- 本仓库惯例：**不自动 git commit**，除非用户明确要求
- 注释与文案用中文，遵循项目现有风格

## 测试影响盘点（先读，再动工）

重排会触碰的断言（重排后必须仍通过）：

| 脚本 | 断言 | 要求 |
|------|------|------|
| `test_guard.js:160` | `<h2>守护</h2>` | 卡③ 标题保持「守护」 |
| `test_guard.js:161` | `id="remind_sedentary_minutes" type="number" min="30" max="120"` | Task 1 改 min="1" 时**同步改此断言** |
| `test_guard.js:131` | `Math\.max\(30, parseInt\(` | Task 1 改钳制时**同步改此断言** |
| `test_shortcut.js:31-32` | hint 含 `Alt+Shift+N 显隐主窗`、`Alt+Shift+P 暂停/恢复监控` | 卡③ 一行 hint 必须保留这两个短语 |
| `test_remind.js:164` | `id="remind_offwork_enabled"` | ID 保留 |
| `test_insights.js:110` | `id="overtime_exclude_remote"` | ID 保留 |
| `test_week_bill.js:91` | `id="billStyleSeg"` | ID 保留 |
| `test_topnav.js` 等 5 处 | app.js 引用的全部 `$()` id 存在于 index.html | 所有 ID 保留即可 |

---

### Task 1: 久坐阈值下限放宽 30 → 1（TDD）

**Files:**
- Modify: `scripts/test_guard.js:131,161`（两处断言）
- Modify: `scripts/test_settings.js`（新增行为断言块）
- Modify: `frontend/app.js:221-223`（readCfg 钳制）
- Modify: `frontend/index.html:389`（min 属性）

**Interfaces:**
- Consumes: 无
- Produces: `readCfg()` 对 `remind_sedentary_minutes` 的钳制区间变为 `[1, 120]`；输入框 `min="1"`

- [ ] **Step 1: 先改断言（红）**

`scripts/test_guard.js` 两处：

```js
// L131 原：
ok("readCfg 阈值钳制下限 30", /Math\.max\(30, parseInt\(/.test(appSrc));
// 改为：
ok("readCfg 阈值钳制下限 1", /Math\.max\(1, parseInt\(/.test(appSrc));

// L161 原：
ok("阈值输入 30–120", /id="remind_sedentary_minutes" type="number" min="30" max="120"/.test(htmlSrc));
// 改为：
ok("阈值输入 1–120", /id="remind_sedentary_minutes" type="number" min="1" max="120"/.test(htmlSrc));
```

- [ ] **Step 2: 跑测试确认失败**

Run: `pwsh -NoProfile -Command 'node scripts/test_guard.js'`
Expected: FAIL 2 条（「readCfg 阈值钳制下限 1」、「阈值输入 1–120」）

- [ ] **Step 3: 改实现（绿）**

`frontend/index.html:389`：

```html
<!-- 原 -->
<input id="remind_sedentary_minutes" type="number" min="30" max="120" step="5" />
<!-- 改为 -->
<input id="remind_sedentary_minutes" type="number" min="1" max="120" step="1" />
```

`frontend/app.js:221-223`（readCfg 内）：

```js
// 原
Math.max(30, parseInt($("remind_sedentary_minutes").value, 10) || 50)
// 改为
Math.max(1, parseInt($("remind_sedentary_minutes").value, 10) || 50)
```

- [ ] **Step 4: 跑 test_guard.js 确认通过**

Run: `pwsh -NoProfile -Command 'node scripts/test_guard.js'`
Expected: 全 PASS

- [ ] **Step 5: test_settings.js 加行为断言（防回归）**

在 `scripts/test_settings.js` 的 `== 保存路径 ==` 段之前插入（`store` 对象加一行 `remind_sedentary_minutes: "50",`，与现有 fixture 并列）：

```js
console.log("== 久坐阈值钳制 1–120 ==");
store.remind_sedentary_minutes = "1";
eq("阈值 1 原样保存", api.readCfg().remind_sedentary_minutes, 1);
store.remind_sedentary_minutes = "0";
eq("阈值 0 回退默认 50", api.readCfg().remind_sedentary_minutes, 50);
store.remind_sedentary_minutes = "999";
eq("阈值 999 钳到 120", api.readCfg().remind_sedentary_minutes, 120);
store.remind_sedentary_minutes = "50";
```

- [ ] **Step 6: 跑 test_settings.js 确认通过**

Run: `pwsh -NoProfile -Command 'node scripts/test_settings.js'`
Expected: 全 PASS（新增 3 条）

---

### Task 2: styles.css 新增三个类

**Files:**
- Modify: `frontend/styles.css`（`.hint` 定义后约 L780 插入）

**Interfaces:**
- Consumes: 无
- Produces: `.time-grid`（2×2 时间网格）、`.sub-section`（卡内虚线分节）、`.hint-details`（折叠长说明）——Task 3 的 HTML 引用这三个类

- [ ] **Step 1: 在 `.hint` 块（styles.css L774-779）之后插入**

```css
/* ====== 设置卡内部层级：时间 2×2 网格 / 虚线分节 / 折叠长说明 ====== */
.time-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 10px;
}

/* 合并卡内小节：虚线分隔，呼应 hint 的弱化风格 */
.sub-section {
  border-top: 1px dashed rgba(255, 255, 255, 0.14);
  padding-top: 10px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

/* 折叠式长说明：summary 一行「▸ 详情」，正文沿用 .hint 字号色 */
.hint-details {
  font-size: 11.5px;
  color: #85858b;
  line-height: 1.6;
}
.hint-details summary {
  cursor: pointer;
  color: #98989e;
  user-select: none;
  min-height: 15px;
  display: inline-flex;
  align-items: center;
  gap: 5px;
}
.hint-details summary::before {
  content: "▸";
  font-size: 10px;
  display: inline-block;
  transition: transform 0.15s;
}
.hint-details[open] summary::before {
  transform: rotate(90deg);
}
.hint-details p {
  margin: 6px 0 0;
}
```

- [ ] **Step 2: 语法自检**

Run: `pwsh -NoProfile -Command 'node -e "console.log(\"css written\")"'`
Expected: 无报错输出（CSS 无独立测试，视觉生效在 Task 3 Step 8 目检）

---

### Task 3: index.html 六卡重排

**Files:**
- Modify: `frontend/index.html:357-561`（`viewSettings` 整段重写）

**Interfaces:**
- Consumes: Task 2 的 `.time-grid` / `.sub-section` / `.hint-details` 类；Task 1 的 `min="1"`
- Produces: 6 张语义化卡片；所有既有元素 ID 不变；app.js 零适配（getElementById 与 DOM 位置无关）

**归位总表（旧 → 新）：**

| 旧位置 | 项 | 新卡 |
|--------|-----|------|
| 基础配置 | 月薪、发薪日、当月上班天数+刷新+info、时间×4 | ① 薪资与作息 |
| 加班配置 | 全部 9 项 | ② 加班 |
| 守护 | 全部 4 项 | ③ 守护 |
| 功能监控 | 3 开关 | ④ 数据监控（上半） |
| 应用使用白名单 | 开关+编辑器 | ④ 数据监控（.sub-section） |
| 基础配置 | 时长格式、副标题+自定义、账单风格、托盘卡片 | ⑤ 外观 |
| 基础配置 | 开机自启 | ⑥ 系统与数据（上半） |
| 数据存储 | 保留期+info+整理 | ⑥ 系统与数据（.sub-section） |

- [ ] **Step 1: 重写 `viewSettings` 为六卡结构**

将 index.html 中 `<div class="app hidden" id="viewSettings">` 到对应 `</div>`（L357-561）整段替换为：

```html
<!-- 二级页面：设置（六卡：薪资与作息 → 加班 → 守护 → 数据监控 → 外观 → 系统与数据） -->
<div class="app hidden" id="viewSettings">
  <div class="page-head">
    <h2>设置</h2>
  </div>

  <!-- 卡①：怎么算钱（原「基础配置」拆出） -->
  <section class="card">
    <h2>薪资与作息</h2>
    <div class="row">
      <label>
        <span>月薪（元）</span>
        <input id="monthly_salary" type="number" min="0" step="100" />
      </label>
      <label>
        <span>发薪日（每月几号）</span>
        <input id="payday" type="number" min="1" max="31" />
      </label>
    </div>
    <label>
      <span>当月实际上班天数（留空=自动）</span>
      <input id="workdays_override" type="number" min="1" max="31" placeholder="自动获取" />
    </label>
    <button id="refreshBtn" class="ghost">刷新工作日数据</button>
    <div id="workdaysInfo" class="hint"></div>
    <div class="sub-section">
      <div class="time-grid">
        <label>
          <span>上午上班</span>
          <input id="am_start" type="time" />
        </label>
        <label>
          <span>上午下班</span>
          <input id="am_end" type="time" />
        </label>
        <label>
          <span>下午上班</span>
          <input id="pm_start" type="time" />
        </label>
        <label>
          <span>下午下班</span>
          <input id="pm_end" type="time" />
        </label>
      </div>
    </div>
  </section>

  <!-- 卡②：加班（原「加班配置」，长说明收进折叠） -->
  <section class="card">
    <h2>加班</h2>
    <label class="switch-row">
      <span>启用加班追踪</span>
      <input id="overtime_enabled" type="checkbox" class="switch" />
    </label>
    <label class="switch-row">
      <span>远程会话不计加班</span>
      <input id="overtime_exclude_remote" type="checkbox" class="switch" />
    </label>
    <p class="hint">检测到远程桌面会话时暂停自动加班记录，回到本机后继续</p>
    <label>
      <span>加班起算时间（留空=下班时间）</span>
      <input id="overtime_start" type="time" placeholder="下班时间" />
    </label>
    <label>
      <span>加班费（元/小时）</span>
      <input id="overtime_rate" type="number" min="0" step="5" value="20" />
    </label>
    <label class="switch-row">
      <span>饭补</span>
      <input id="overtime_meal_enabled" type="checkbox" class="switch" />
    </label>
    <label>
      <span>饭补金额（元）</span>
      <input id="overtime_meal" type="number" min="0" step="5" value="20" />
    </label>
    <div class="sub-section">
      <label class="switch-row">
        <span>休息日 / 节假日加班</span>
        <input id="weekend_overtime" type="checkbox" class="switch" />
      </label>
      <div id="restOtFields">
        <label>
          <span>休息日加班起算时间</span>
          <input id="weekend_ot_start" type="time" placeholder="09:00" />
        </label>
        <label>
          <span>休息日费率（元/小时）</span>
          <input id="overtime_rate_weekend" type="number" min="0" step="1" placeholder="同工作日" />
        </label>
        <label>
          <span>法定节假日费率（元/小时）</span>
          <input id="overtime_rate_holiday" type="number" min="0" step="1" placeholder="同休息日" />
        </label>
        <details class="hint-details">
          <summary>详情</summary>
          <p>休息日没有「下班时间」概念，起算时间默认 09:00——沿用工作日的 18:00 会让上午来、下午走的人一分钱算不到。费率留空表示沿用上一级（法定节假日 → 休息日 → 工作日）。调休补班日按工作日规则计算，不受本开关影响</p>
        </details>
      </div>
    </div>
  </section>

  <!-- 卡③：守护（v1.3.0，hint 压一行 + 折叠） -->
  <section class="card">
    <h2>守护</h2>
    <label class="switch-row">
      <span>久坐提醒</span>
      <input id="remind_sedentary_enabled" type="checkbox" class="switch" />
    </label>
    <label>
      <span>连续活跃阈值（分钟）</span>
      <input id="remind_sedentary_minutes" type="number" min="1" max="120" step="1" />
    </label>
    <label class="switch-row">
      <span>下班提醒</span>
      <input id="remind_offwork_enabled" type="checkbox" class="switch" />
    </label>
    <label class="switch-row">
      <span>全局快捷键</span>
      <input id="shortcuts_enabled" type="checkbox" class="switch" />
    </label>
    <p class="hint">Alt+Shift+N 显隐主窗 · Alt+Shift+P 暂停/恢复监控</p>
    <details class="hint-details">
      <summary>详情</summary>
      <p>主窗隐藏时提醒改为系统通知；快捷键与其他软件冲突时可关闭此开关</p>
    </details>
  </section>

  <!-- 卡④：数据监控（原「功能监控」+「应用使用白名单」合并） -->
  <section class="card">
    <h2>数据监控</h2>
    <label class="switch-row">
      <span>鼠标键盘活动监控</span>
      <input id="monitor_activity" type="checkbox" class="switch" />
    </label>
    <label class="switch-row">
      <span>应用使用监控</span>
      <input id="monitor_app_usage" type="checkbox" class="switch" />
    </label>
    <label class="switch-row">
      <span>媒体播放监控</span>
      <input id="monitor_audio" type="checkbox" class="switch" />
    </label>
    <p class="hint">监控在后台持续运行会占用少量 CPU，关闭不影响已统计数据</p>
    <div class="sub-section">
      <label class="switch-row">
        <span>仅统计白名单内应用</span>
        <input id="app_whitelist_enabled" type="checkbox" class="switch" />
      </label>
      <details class="hint-details">
        <summary>详情</summary>
        <p>关闭或名单为空 = 统计全部前台应用；开启后只统计下方列出的应用（按展示名、大小写不敏感，如「微信」「钉钉」「VS Code」）</p>
      </details>
      <div class="wl-editor">
        <div class="wl-input-row">
          <input id="appWhitelistInput" type="text" placeholder="输入应用名后点添加" />
          <button id="appWhitelistAdd" class="primary wl-add">添加</button>
        </div>
        <div id="appWhitelistList" class="wl-list"></div>
      </div>
    </div>
  </section>

  <!-- 卡⑤：外观（原「基础配置」拆出） -->
  <section class="card">
    <h2>外观</h2>
    <label>
      <span>时长显示格式</span>
      <select id="duration_format">
        <option value="hms">几小时几分几秒（如 5小时10分30秒）</option>
        <option value="hm">几小时几分（如 5小时10分）</option>
        <option value="h">小数小时（如 5.2h）</option>
      </select>
    </label>
    <label>
      <span>首页副标题文案</span>
      <select id="tagline_style">
        <option value="dynamic">动态状态（随开工 / 快下班自动变）</option>
        <option value="price">你今天的每一分钟，都明码标价</option>
        <option value="rise">每一秒，钱都在涨</option>
        <option value="count">搬砖的每一分钟，都算数</option>
        <option value="classic">实时计算你今天赚了多少钱（原版）</option>
        <option value="none">不显示这一行</option>
        <option value="custom">自定义…</option>
      </select>
    </label>
    <label id="taglineCustomRow">
      <span>自定义文案</span>
      <input id="tagline_custom" type="text" maxlength="40" placeholder="写你想看到的那句话" />
    </label>
    <details class="hint-details">
      <summary>详情</summary>
      <p>副标题显示在主界面顶部。选「动态状态」会随未开工、午休、快下班（剩 30 分钟内）、已下班、休息日自动切换</p>
    </details>
    <div class="switch-row">
      <span>账单风格</span>
      <div class="mon-seg" id="billStyleSeg" role="tablist" aria-label="账单风格切换">
        <button class="mon-seg-item" data-bill="receipt">小票</button>
        <button class="mon-seg-item" data-bill="dashboard">仪表盘</button>
      </div>
    </div>
    <p class="hint">小票 = 晒单情绪；仪表盘 = 数据复盘。切换后返回账单页生效</p>
    <label class="switch-row">
      <span>托盘悬停显示彩色卡片</span>
      <input id="tray_hover_card" type="checkbox" class="switch" />
    </label>
  </section>

  <!-- 卡⑥：系统与数据（原「数据存储」+ 基础配置的开机自启合并） -->
  <section class="card">
    <h2>系统与数据</h2>
    <label class="switch-row">
      <span>开机自动启动</span>
      <input id="launch_on_boot" type="checkbox" class="switch" />
    </label>
    <p class="hint">写入系统启动项（注册表 Run），登录 Windows 时自动在托盘运行</p>
    <div class="sub-section">
      <label>
        <span>历史数据保留</span>
        <select id="retention_days">
          <option value="0">永久保留（默认）</option>
          <option value="180">半年</option>
          <option value="365">1 年</option>
          <option value="730">2 年</option>
        </select>
      </label>
      <div id="storageInfo" class="storage-box"><span class="hint">读取中…</span></div>
      <button id="cleanupBtn" class="ghost">立即整理</button>
      <details class="hint-details">
        <summary>详情</summary>
        <p>每天首次启动会自动整理一次。写前日志（WAL）是 SQLite 的临时写入区，程序常驻不退出时它不会自动收缩，整理后通常能从数 MB 降到几十 KB；图标缓存超过 180 天会重新提取。以上都不影响你的数据，只有「保留期」会真正删除历史记录</p>
      </details>
    </div>
  </section>
</div>
```

要点（执行者必查）：
- 原文里「开机自启」的 hint 有一句「也可随时在任务管理器『启动』页关闭」被并入上面一行 hint 的语义，此处压缩掉——如需保留可放回，但必须保持一行
- `taglineCustomRow` 的显隐由 app.js 控制，初始不 hidden，保持原样
- 旧「功能监控」「数据存储」「基础配置」「加班配置」「应用使用白名单」五张卡的 `<section>` 整体删除，防止 ID 重复

- [ ] **Step 2: 跑设置页相关的全部脚本**

Run: `pwsh -NoProfile -Command 'node scripts/test_topnav.js; node scripts/test_guard.js; node scripts/test_remind.js; node scripts/test_shortcut.js; node scripts/test_insights.js; node scripts/test_week_bill.js; node scripts/test_settings.js; node scripts/test_hidden.js'`
Expected: 全部 0 failed（任何一个 FAIL 都先查「测试影响盘点」表）

- [ ] **Step 3: 启动 debug 版目检**

Run: `pwsh -NoProfile -Command 'Start-Process "c:\Users\Tim\Work\my-projects\niuma-timer\bin\debug\niuma-timer.exe"'`
Expected: 设置页呈现 6 张卡、顺序为 薪资与作息→加班→守护→数据监控→外观→系统与数据；四处「▸ 详情」可展开/收起且不撑破布局；时间 2×2；虚线分节正常

---

### Task 4: 全量回归 + CHANGELOG + 手测清单

**Files:**
- Modify: `CHANGELOG.md`（[未发布] 区加一条）
- Test: `scripts/run_all.js`

**Interfaces:**
- Consumes: Task 1-3 全部产出
- Produces: 全绿回归 + 可交付的 debug 构建

- [ ] **Step 1: 全量回归**

Run: `pwsh -NoProfile -Command 'node scripts/run_all.js'`
Expected: 19 个脚本全部 0 failed

- [ ] **Step 2: build 冒烟**

Run: `pwsh -NoProfile -Command '& .\build.bat debug'`（在仓库根目录）
Expected: 构建成功，`bin\debug\niuma-timer.exe` 更新

- [ ] **Step 3: CHANGELOG 记录**

在 `CHANGELOG.md` 的 `[未发布]` 区（v1.3.0 四条之后）追加：

```markdown
- **设置页信息架构重组**：拆解「基础配置」大杂烩，六张语义化卡片（薪资与作息 / 加班 / 守护 / 数据监控 / 外观 / 系统与数据）；长说明收进「详情」折叠；久坐阈值下限放宽到 1 分钟
```

- [ ] **Step 4: 向用户输出手测清单**

文字输出（不改代码）：6 张卡各改一项验证 config.json 持久化；4 处「详情」折叠展开收起；阈值输入 1 分钟可保存；白名单增删、账单风格切换、副标题选「自定义」出输入框。

---

## 自查记录

- **Spec 覆盖**：设计文档 §3 六卡 → Task 3（含每卡完整 HTML）；§4 降噪四处 → Task 3 卡②③④⑤⑥ 的 `<details>`；§5 层级（time-grid/sub-section/switch-row）→ Task 2+3；§6 阈值 → Task 1；§7 文件表 → Tasks 1-3；§8 测试 → Task 1 Step 5、Task 4、手测清单
- **占位符**：无 TBD/TODO；所有代码步骤给出完整代码
- **类型/命名一致**：`.time-grid` `.sub-section` `.hint-details` 在 Task 2 定义、Task 3 引用，拼写一致；`min="1"` 与 Task 1 断言一致
