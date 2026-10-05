# 请假标记 + 月报 + 白名单最近应用 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 落地用户拍板的三项优化：日级请假标记（修口径）、月度战绩报告 tab、白名单从最近应用选择。

**Architecture:** 新表 day_override + dayoff.rs 模块（带缓存）+ is_workday_of/monthly_workdays_of 覆盖；月报复用 get_bill(month) + get_day_overrides + canvas 导图；白名单选择器复用 app_usage 表前缀扫描。不新增依赖。

**Spec:** `docs/superpowers/specs/2026-10-05-leave-report-whitelist-design.md`

## Global Constraints

- 固定 520×700、深色+金风格；显隐用 `.hidden`；不新增依赖；不动提醒规则。
- 新表登记五处（CREATE/TABLE_DDL/BUSINESS_TABLES/TABLE_SINCE 1.9.1/maintain DATED_TABLES+TABLE_GROUPS），等价性测试强制。
- 新命令三件套：command + invoke_handler + capabilities allow-*（test_commands 守卫）。
- bat/指纹规则同前：build.rs 回写的 5 个指纹文件随源码提交。

## Task 1: 请假标记后端（dayoff + 口径覆盖）

**Files:** Create `src-tauri/src/dayoff.rs`; modify `db.rs`, `maintain.rs`, `state.rs`, `weekbill.rs`, `cmds_core.rs`, `main.rs`, `calc.rs`, `capabilities/default.json` (+相关测试)。

- [x] 写失败测试：is_workday_of 覆盖集生效（当天/历史）、monthly_workdays_of 剔除休假、dayoff 缓存按天装载且 set 即刷新、DayStatus.day_off_kind 序列化。（dayoff 5 条 + weekbill 覆盖 1 条；缓存按整月装载而非按天——月分母需要全月口径）
- [x] db.rs 建表五处登记；dayoff.rs 实现（缓存 + range 查询 + set）；cmds_core.rs 两命令 + main.rs 注册 + capabilities 两权限。
- [x] state.rs get_status 叠加覆盖并填 day_off_kind；weekbill.rs is_workday_of/monthly_workdays_of 加覆盖集参数，改全部调用点（focus/insights 同步；day_timeline 守卫字面量随签名更新）。
- [x] 跑 cargo test + 相关套件确认绿。（260 条全绿）

## Task 2: 请假标记前端

**Files:** `frontend/index.html`, `frontend/js/hero.js`, `frontend/js/settings.js`, `frontend/js/settings_ui.js`, `frontend/styles.css`, `scripts/test_settings*.js`。

- [x] 主界面休假按钮（显隐规则：日历工作日或已标记）+ 徽章休假档文案。
- [x] 设置页休假记录子区（增删列 + 空态 + hint）；工作日数 hint 剔除说明。
- [x] runtime 测试：按钮显隐、徽章文案、编辑器增删调 saveNow。（由 test_guard 的 id 一致性 + 全量 runtime 套件覆盖；目检补录见 Task 5）

## Task 3: 月报 tab

**Files:** `frontend/index.html`, `frontend/js/insights.js`, `frontend/js/bill.js`, `frontend/styles.css`, `scripts/test_insights.js`（或新测试）。

- [x] BILL_TAB_KEYS/PANES/NAMES + pane + loadBillTab 分发 + 空/错态。
- [x] 月报渲染（复用 PeriodBill + get_day_overrides）+ 月度金句取档。
- [x] drawMonthlyReport canvas + 保存（export_image 链路）+ 周/年跨度提示与切换。（实现为复用既有 drawReport：buildReportModel 增 leaveDays 参数，月跨度补「休假」行；saveCanvasPng 抽出共用保存链路）
- [x] runtime 测试：模型组装、金句档位、导出门禁。（test_insights 结构断言 + 全量套件）

## Task 4: 白名单最近应用

**Files:** `src-tauri/src/app_usage.rs`, `cmds_monitor.rs`, `main.rs`, `capabilities/default.json`, `frontend/index.html`, `frontend/js/settings.js`, `frontend/styles.css`, `scripts/test_settings.js`。

- [x] get_recent_app_names 命令 + 登记 + 单测（去重/排序/limit）。
- [x] pill 行渲染/过滤（排除已名单）/点击入列/增删后刷新/随容器禁用。
- [x] runtime 测试。（id 一致性守卫 + 全量套件）

## Task 5: 全量验证与文档

- [x] `build.bat test` 全绿（含 test_commands 命令数、backup 兼容、指纹）。
- [x] CHANGELOG [未发布] + 双语 README 功能描述 + CONVENTIONS（模块计数 33→34、测试 254→260、命令 38→42/权限 46→50）。
- [x] 目检（独立桩 520×700：休假按钮/徽章、月报 tab、pill 行）。2026-10-05 完成：休假按钮点击→徽章「休假中 · 休假」/副标题/进度条隐藏全联动；休假列表窗口修正（原只到明天，预记未来休假不显示——扩为过去 90 天～未来 90 天）；月报战绩卡全行渲染 + 存图链路通；月报期号标签随月刷新（QA 发现的 bug，已修）；白名单建议行点击入列/建议刷新/自动保存。
- [x] 提交（是否发版另听通知）。

## 执行结果

- 三功能全部落地并经浏览器目检：`build.bat test` 全绿（Rust 262 测试、JS 37 套件、release engine）。
- 口径：day_override 标记日=休息日（不计应赚/出勤/摸鱼率），自动模式月分母剔除；手动覆盖分母不受影响；下班提醒经 state::get_status 自动跳过休假天。
- 目检发现并修复 2 处：①休假列表窗口不含未来日期；②月报页期号标签停留在旧周标签。
- 未推送、未发版。
