# 摸鱼洞察 C1：摸鱼成本货币化

2026-09-13

## 背景

上一轮「摸鱼统计」（commit f7c9b7e）已把应用归入工作 / 摸鱼 / 沟通 / 其他四类，
应用明细页有「今日构成」卡，主页监控卡有一行摸鱼速览。但分类数据还停留在
**时长**层面，没有和这个软件的灵魂——「钱」——挂钩。

本轮（C 轴 Top 方案中的 C1）把摸鱼时长换算成钱，和 hero 卡里跳动的已赚金额
形成「一边赚、一边烧」的实时对照。

## 目标

- 打开主窗口第一眼就能看到「今天摸掉了多少钱」，且随 2 秒轮询实时跳动
- 文案有牛马主题的损味，制造截图传播点
- 应用明细的四类构成补上金额口径

## 非目标（明确不做）

- **不动 Rust、不加 IPC**：全部数据前端已有（`get_app_usage_summary` 的
  `categories` 四类秒数 + `get_status_cmd` 的 `hourly_rate`）
- 不做「净到手 = 已赚 − 摸掉」（摸鱼时间公司也付了钱，减法在概念上误导，评审中砍掉）
- 不做跨天 / 周报趋势（C3，与 B 轴周报重叠）
- 不做逐小时分类堆叠（C2，需后端 per-hour 分类聚合）
- 不加通知、不加设置项、不引入图标库

## 数据口径

- **摸鱼成本** = 摸鱼类秒数 ÷ 3600 × 时薪（`hourly_rate`，元/小时）
- **摸鱼率** = 摸鱼秒数 ÷ 四类（work+slack+comm+other）总前台秒数 × 100%
  - 分母用「被统计到的前台时间」，不含离开电脑 / 无前台应用的空档，避免把
    午休、开会（没碰电脑）算成摸鱼
- 复用现有 `summary.categories`（固定序 work/slack/comm/other，见
  app_usage.rs `fold_categories`），分类为查询时归类、用户改分类即时重算

### 显示守卫

烧钱行满足**全部**条件才显示，否则整行加 `hidden`：

1. 工作日（`is_workday`，与赚钱进度条同一守卫）
2. 时薪 > 0（未配月薪时不显示，避免 ¥NaN / 一串 0）
3. 当日四类总时长 > 0（没有任何应用记录时不评判）

摸鱼为 0 时金额显示 `¥0.00` 且**不隐藏**（正向激励，不是数据缺失）。

## 界面改动

### 1. hero 卡烧钱行（index.html · `.live`）

位置：赚钱大数字 + 赚钱进度条之下，分隔细线之下：

```
            ¥328.50                 ← 已有金色大数字
     全天应赚 ¥500.00 · 65.7%       ← 已有进度条
     ─────────────────────────
     🐟 摸掉 ¥46.20 · 摸鱼率 12%    ← 新增
                    老板的梦中情马  ← 损味文案（右侧）
```

新增结构（id 约定与现有 lp-* 保持同一命名风格，slack-* 前缀）：

```html
<div class="slack-burn hidden" id="slackBurn">
  <div class="sb-main">
    <span class="sb-label">🐟 摸掉</span>
    <b class="sb-amt" id="sbAmt">¥0.00</b>
    <span class="sb-rate" id="sbRate">摸鱼率 0%</span>
  </div>
  <span class="sb-quip" id="sbQuip"></span>
</div>
```

### 2. 损味文案四档（app.js）

按摸鱼率分档（边界：10 / 25 / 40，左闭右开，`pct < 10` 起）：

| 摸鱼率 | 文案 |
|---|---|
| `< 10%` | 老板的梦中情马 |
| `10–25%` | 摸得克制，装得敬业 |
| `25–40%` | 快三分之一的班白上了 |
| `≥ 40%` | 老板看完连夜注销公司 |

抽成纯函数 `slackQuip(pct)` 便于单测阈值边界。
无应用记录（总时长 0）时烧钱行整体隐藏，不显示文案。

### 3. 应用明细「今日构成」金额化（app.js · renderCategories）

四类现有：色点 + 中文标签 + 时长。每格时长下方补一行等宽小字金额
（该类秒数 ÷ 3600 × 时薪）：摸鱼类橙红加粗（`cat-c-slack` 同色），
其余三类灰色次要文字。时薪不可用时不渲染金额行（时长构成仍正常）。

## 实现位置（app.js）

- tick()：已有 `get_status_cmd` 状态 `s`；摸鱼数据来自应用 summary 缓存
  （主页监控卡 app 面板的现有数据源）。在渲染监控卡摸鱼速览行的同一处
  计算并填充烧钱行，避免新增 IPC 调用
- 时薪取 `s.hourly_rate`；分类秒数取最近一次 app usage summary 的
  `categories`（无数据时四类秒数为 0，走隐藏守卫）
- 休息日 / 时薪 0 时同样不得残留旧数字：隐藏时不更新文本，显示前先刷新

## 测试计划

扩展 `scripts/test_slacking.js`（静态断言，Node 直跑）：

- 烧钱行三元素 id（slackBurn / sbAmt / sbRate / sbQuip）存在于 index.html
- 存在纯函数 `slackQuip`，四档文案字面量各被断言一次
- 计算口径断言：摸鱼成本公式（÷3600 × hourly_rate）、摸鱼率分母四类求和
- 三条显示守卫（is_workday / hourly_rate > 0 / 总时长 > 0）对应代码分支
- 构成金额化：renderCategories 中金额行渲染、摸鱼类高亮 class
- `$()` 兜底：app.js 新引用的 id 全部存在于 index.html
- 全量回归 10 个测试文件 + `.\build.bat debug`（build.rs 自动同步缓存戳）

## 涉及文件

| 文件 | 改动 |
|---|---|
| `frontend/index.html` | `.live` 卡内新增烧钱行 |
| `frontend/styles.css` | `.slack-burn/.sb-*` 样式（分隔线、橙红金额、文案弱化） |
| `frontend/app.js` | slackQuip 纯函数 + tick 填充 + 守卫 + renderCategories 金额 |
| `scripts/test_slacking.js` | 新增上述断言 |
| `CHANGELOG.md` | 未发布·新增 区补条目 |

零 Rust 文件改动。
