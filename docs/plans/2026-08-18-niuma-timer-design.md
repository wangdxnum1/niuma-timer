# 牛马计时器（NiuMa Timer）设计文档

- 日期：2026-08-18
- 平台：Windows（暂定仅 Windows）
- 形态：系统托盘常驻工具，单文件 `.exe`
- 技术栈：**Tauri v2（Rust 后端 + 原生 WebView 前端）**

---

## 1. 目标与范围

实时告诉用户「今天赚了多少钱、上了多少小时班、距下班还有多久」。

- 只统计**当天**，不累计历史天数（零点按本地日期自动重置）。
- 计时窗口由用户配置：上午段 + 下午段，午休不计时。
- 下班即封顶：过了下午下班时间停止累计，已工作封顶为当日总工时，距下班显示「已下班」。
- **加班功能本期预留不实现**（后续可加「加班从 X 点起计」）。

---

## 2. 技术栈与理由

| 关注点 | 选型 | 理由 |
|--------|------|------|
| 框架 | Tauri v2 | 用户指定；原生托盘 + WebView 前端，设置页用 HTML 写起来快 |
| 后端语言 | Rust（stable） | 性能、单文件分发 |
| 托盘 | Tauri v2 托盘 API（`app.with_tray_icon` / `tauri-plugin-tray-icon`） | Win32 原生，hover tooltip + 右键菜单 |
| 前端 | 原生 HTML/CSS/JS（无框架） | 设置页仅一个表单，无需引入构建链 |
| 网络 | `reqwest`（async，Tauri 自带 tokio 运行时） | 拉取节假日数据 |
| 时间 | `chrono` | 日期/时段计算 |
| 配置持久化 | 直接 `fs` 写 JSON 到 `app.path().app_config_dir()` | 透明、无额外插件 |
| JSON | `serde` + `serde_json` | 配置与节假日解析 |

> 注：Tauri 在 Windows 依赖系统 WebView2（Win10/11 通常已预装）。分发时可为未安装环境附带引导。

---

## 3. 项目结构

```
niuma-timer/
├─ src-tauri/
│  ├─ Cargo.toml
│  ├─ tauri.conf.json
│  ├─ build.rs
│  ├─ icons/                 # 托盘图标 + 窗口图标 (icon.ico / .png)
│  ├─ capabilities/default.json
│  └─ src/
│     ├─ main.rs             # 入口：建托盘、起定时任务、注册命令
│     ├─ config.rs           # 配置结构 + 读写
│     ├─ calc.rs             # 核心计时/算钱逻辑
│     ├─ holiday.rs          # 节假日拉取/缓存/当月工作日统计
│     └─ tray.rs             # 托盘图标、tooltip、菜单
├─ frontend/
│  ├─ index.html             # 设置窗口页面
│  ├─ styles.css
│  └─ app.js                 # 表单 <-> Tauri 命令
└─ docs/plans/2026-08-18-niuma-timer-design.md
```

---

## 4. 配置模型（config.json）

```json
{
  "monthly_salary": 15000,
  "am_start": "09:00",
  "am_end":   "12:00",
  "pm_start": "13:00",
  "pm_end":   "18:00",
  "workdays_override": null,   // 手动覆盖当月实际上班天数；null = 用自动算
  "payday": 10,                // 发薪日（每月几号），用于“距发薪日”倒计时
  "last_holiday_year": 2026    // 缓存年份标记
}
```

- 每日工时 = (am_end − am_start) + (pm_end − pm_start)，例如 3h + 5h = 8h
- 当月实际上班天数：
  - 若 `workdays_override != null` → 用覆盖值
  - 否则 → 由 `holiday.rs` 解析当月「班/补班」天数
- 时薪 = 月薪 ÷ 当月实际上班天数 ÷ 每日工时

---

## 5. 核心计算（calc.rs，每秒一次）

输入：当前本地时间 `now`、配置、当日是否为工作日。

```
is_workday = holiday::is_workday(today)   // 班/补班=true；休/法定=false
if !is_workday:
    return { earned: 0, worked_h: 0, to_off: "今天休息" }

seg_am = overlap(now, [am_start, am_end])
seg_pm = overlap(now, [pm_start, pm_end])
worked_h = (seg_am + seg_pm) in hours

if now >= pm_end:
    worked_h = daily_hours              // 封顶为整天
    to_off   = "已下班"
else:
    to_off   = (pm_end - now) in hours

rate_per_min = hourly_rate / 60               // 实时赚钱速率（元/分钟）
days_to_pay  = days_to_payday(today, payday)  // 距下一个发薪日天数

earned = worked_h * hourly_rate
```

- `overlap` 只取落在时段内的部分，午休（am_end→pm_start）天然不计。
- 零点前 `now < am_start`：worked_h=0，to_off = 距 pm_end 的时长（即「距下班还有 Xh」）。
- 所有时间用本地时区（`chrono::Local`）。

---

## 6. 托盘行为（tray.rs）

- 启动即创建托盘图标（常驻，无主窗口可见）。
- **悬停 tooltip 实时显示**（每秒刷新，控制在 127 字符内）：
  `今天已赚 ¥XX.XX ｜ 已工作 X.Xh ｜ 距下班 X.Xh ｜ ¥X.XX/分 ｜ 距发薪 N天`
  （非工作日显示「今天休息 ｜ ¥X.XX/分 ｜ 距发薪 N天」）
- **托盘图标动态文字**：每秒把「今天已赚 ¥XX」渲染到 64×64 图标上（见 §11.3），关键数字常驻可见，免悬停。
- **右键菜单**：
  - 设置 → 显示/聚焦设置窗口
  - 刷新工作日 → 立即重拉节假日数据
  - 退出 → `app.exit()`
- tooltip 文本通过 `tray.set_tooltip()` 每秒更新（Windows 托盘标题上限约 127 字符，足够）。

> 可选增强（本期不做）：将「今天已赚」数字绘制到托盘图标本身，更醒目。

---

## 7. 节假日数据（holiday.rs）

数据源：`https://timor.tech/api/holiday/year/{YEAR}`（免费，返回全年每日类型：
`0`=工作日/`1`=周末/`2`=补班/`3`=法定节假日）。

流程：
1. 启动 / 点「刷新」/ 每日 00:05 自动：拉取**当前年**数据。
2. 解析当月所有日期，`type ∈ {0,2}` 记为实际上班天数。
3. 写入缓存 `app_config_dir()/holiday_{year}.json`（含拉取时间戳）。
4. 断网或拉取失败 → 读取本地缓存；无缓存 → 回退到「当月自然日−周末」粗算并提示。

`monthly_workdays()`：优先 `workdays_override` → 否则用当年缓存统计；无缓存触发一次拉取（失败用粗算）。

---

## 8. 设置窗口（frontend + 命令）

- 前端：`index.html` 表单（月薪、四个时间点、发薪日、手动覆盖天数、刷新按钮），并实时展示「¥/分 速率」与「距发薪 N 天」。
- 加载时调用 `load_config` 命令回填；保存时调用 `save_config` 写盘并热更新内存状态。
- 「刷新工作日」按钮调用 `refresh_holidays` 命令，返回当月实际上班天数并显示。
- Tauri 命令（Rust 侧 `main.rs` 注册）：
  - `load_config() -> Config`
  - `save_config(cfg) -> ()`
  - `refresh_holidays() -> u32`（返回当月上班天数）
  - `get_today_status() -> Status`（供前端实时预览，可选）

窗口策略：启动时主窗口 `visible:false`；菜单「设置」调用 `window.show()` + `set_focus()`；关闭按钮仅隐藏（`prevent_close`）保持托盘运行。

---

## 9. 定时与状态共享

- 全局状态：`Arc<Mutex<AppState>>`，含 `config` 与 `holiday_cache`。
- 计时任务：Tauri 启动后用 `std::thread` 起一个 1s 循环（或 `tauri::async_runtime::spawn` + `sleep`），计算后 `tray.set_tooltip()`。
- 配置变更（save_config）写 `Mutex` 并落盘，下一秒循环即生效。
- 每日自动刷新：循环内检测本地日期跨天 → 触发节假日刷新 + 重置。

---

## 10. 构建与分发

```bash
cd src-tauri
cargo build --release        # 产出 src-tauri/target/release/niuma-timer.exe
```
- 图标：准备 `icon.ico`（托盘用 16/32px）与窗口图标。
- 可选安装包：`cargo tauri build` 配合 `tauri.wix` 模板出 `.msi`。
- 可选：未装 WebView2 环境运行时提示安装。

---

## 11. 增强：爽感三件套（本期实现）

### 11.1 实时赚钱速率
- `rate_per_min = hourly_rate / 60`（元/分钟），配置不变则恒定。
- 显示「¥X.XX/分」，出现在托盘 tooltip 与设置页；可同时给「¥X.XX/秒」「¥X.XX/时」。

### 11.2 距离发薪日倒计时
- 新增配置 `payday`（每月几号，默认 10）。
- `days_to_pay = 下一个 payday 距今天数`（今天已过本月 payday 则算下月）。
- 显示「距发薪 N 天」，tooltip + 设置页可改发薪日。

### 11.3 托盘图标动态文字
- 用 `image` 生成 64×64 RGBA 缓冲，`fontdue`（内嵌轻量 .ttf 字体）将「¥XX」光栅化居中绘制，深色底 + 亮色字。
- 逐秒 `tray.set_icon(RgbaIcon{...})` 更新，关键数字常驻可见。
- 依赖：`image`、`fontdue`（或 `ab_glyph`）。Tauri 托盘支持传 RGBA 原始图标。

---

## 12. 本期不做（更新）

- 加班计时（「加班从 X 点起计」）→ 预留。
- 下班提醒弹窗 / 提示音 / Windows 开机自启（属「提醒与自启」增强包，本期未选）。
- 本周/本月累计收益趋势、进度条、CSV 导出（属「数据与回顾」增强包，本期未选）。
- 税后时薪 / 摸鱼成本（属「进阶计算」增强包，本期未选）。
- 跨平台（macOS/Linux）。

---

## 13. 验收标准

1. 程序启动后托盘出现，悬停实时显示「今天已赚/已工作/距下班/¥每分/距发薪」。
2. 托盘图标可见「今天已赚 ¥XX」数字，每秒刷新。
3. 设置里改月薪/时间段/发薪日，保存后 tooltip 与图标立即按新值变化。
4. 点「刷新工作日」能拉到当年数据并显示当月实际上班天数。
5. 跨过下午下班时间后，已工作封顶、距下班显示「已下班」；午休时段不计。
6. 非工作日显示「今天休息」，已赚为 0。
7. 零点自动重置为当天新值；距发薪日随日期推进更新。
8. 断网时能用本地缓存的节假日数据。
9. `cargo build --release` 成功产出单文件 exe。
