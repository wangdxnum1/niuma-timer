# 设置页 UI 与内容优化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 落地用户确认的六组设置逐项原型，让设置易找、规则易懂、反馈可靠。

**Architecture:** 使用现有设置 DOM、配置字段和持久化流程。新增 `settings_ui.js` 集中表现层模型、校验、联动和导航；`settings.js` 保留读写配置并串行处理保存。只为准确计薪预览给已有 DayStatus 增加真实工作日数，不新建 IPC。

**Tech Stack:** Tauri 2 / Rust / 原生 HTML、CSS、JS / Node runtime 测试。

**Spec:** `docs/superpowers/specs/2026-10-04-settings-ui-redesign-design.md`

## Global Constraints

- 固定 520×700 窗口，现有深色与金色风格；不得新增依赖、数据库表或业务命令。
- 保留配置字段和原值；显隐使用 `.hidden`，自动保存须经过 configLoaded 门闸。
- 不使用原型示例日期、版本、金额；0 费率不得视为空。
- 不更改后台计薪/加班/提醒规则，不发版，不操作真实备份。
- 保留起始工作区五个缓存指纹文件改动；在 `codex/settings-ui-redesign` 分支实施，不 stash。
- 本会话已明确授权写文档后直接实施，采用当前会话逐项执行，无额外设计或执行方式确认。

## Review Focus

1. 加载失败或异步保存失败不能显示已保存，也不能覆盖磁盘旧配置（Task 3）。
2. 时薪模式、关闭加班/餐补/监控后再打开必须保留原值；发薪日继续可见（Task 2–3）。
3. 原配置中过期工作日覆盖与非预置保留期不得在加载/切页时被改写（Task 1、3）。
4. 连续输入、点击分段、离开设置页产生的异步请求必须串行且保存最终意图（Task 3）。
5. 非法时间、0 费率、未取得真实工作日、空白名单必须准确反馈而不伪造默认（Task 1–3）。

## Task 1: 表现层模型与回归测试

**Files:** Create `frontend/js/settings_ui.js`, `scripts/test_settings_ui.js`; modify `scripts/lib/fe_sources.js`, `frontend/index.html`, `src-tauri/src/calc.rs` and conventions module counts.

**Interfaces:** Produces `settingsScheduleModel(cfg, monthlyWorkdays) -> { valid, hours, hourlyRate, dailyPay, message }`, `settingsRateModel(cfg) -> { weekend, holiday }`, `settingsValidationErrors(cfg) -> Array<{id,message}>`, `settingsOverrideIsCurrent(cfg, yearMonth) -> boolean`. Consumes existing `minutesOf` and runtime values, no DOM in these model functions.

- [x] Write runtime assertions: 09–12/13–18 = 8 hours; inverted/overlapping times invalid; equal single segment allowed; missing workdays => unknown monthly rate; hourly mode independent of days; override current/expired; blank fee inherits, zero remains zero; range validation respects enabled fields.
- [x] Run `node scripts/test_settings_ui.js`, confirm new functions absent/failing.
- [x] Implement model functions, load chunk after settings.js, before hero.js; CHUNKS register exactly once. Add `monthly_workdays` to DayStatus plus serialization test or existing structure assertions.
- [x] Run `node scripts/test_settings_ui.js` and frontend split tests, confirm pass.

## Task 2: 六组设置 DOM、样式与导航

**Files:** Modify `frontend/index.html`, `frontend/styles.css`, `frontend/js/settings_ui.js`; extend runtime tests only for meaningful behavior.

**Interfaces:** Produces `initSettingsUI()`, `refreshSettingsUI()`, `setSettingsSaveState(state, message?)`, `setWorkdaysModeUI(manual)`; consumes models in Task 1 and existing DOM configuration controls.

- [x] Replace only settings section, preserving existing input IDs, six cards, debug card, and backup bindings; add named navigation and sticky header.
- [x] Add scoped CSS for stacked fields, units, previews, disabled states, error messages, subgroups and shortcut keycaps. Keep 35px controls, 7px radius, 12px cards; reflow narrow layouts.
- [x] Bind category scrolling, all input previews and opening dependencies. Add manual/auto workdays UI, effective rate hints, duration/tagline previews and character count. Keep payday outside monthlySalaryRow.
- [x] Exercise selectors, actual dependent DOM changes, invalid field feedback and keyboard navigation in runtime/DOM harness; run targeted settings, tagline, focus, storage and frontend split tests. 选择器、字段反馈及相关套件已验证；浏览器依赖显隐与键盘操作目检已于 2026-10-05 补齐（见 Task 4 目检记录）。

## Task 3: 配置联动、校验与可靠保存

**Files:** Modify `frontend/js/settings.js`, `frontend/js/hero.js`, `frontend/js/core.js`, `frontend/js/boot.js`, `frontend/js/overtime.js`, `frontend/js/storage.js`; tests `scripts/test_settings_ui.js`, `scripts/test_settings.js`.

**Interfaces:** Consumes `refreshSettingsUI`, model validation and `setSettingsSaveState`. Produces serial `doSave({silent=false}) -> Promise<boolean>` plus meaningful loading/error/success status; leaves `readCfg` field semantics compatible.

- [x] Add failing tests for queued saves, failed retry, blocked load and expired override. Existing test VM contexts receive explicit helper stubs when extracting functions in isolation.
- [x] Load values before refreshing UI; expire stale override in UI without writing on load. Restore nonstandard retention values. Feed previews `lastStatus.monthly_workdays`, refresh results and current form values.
- [x] Validate enabled fields before enqueueing saves; serial calls update only successful snapshot, and newest intent determines status. Do not clear values merely to hide controls.
- [x] Scope homepage monitoring segment selection to its own container so it cannot deselect salaryModeSeg. Use classList for rest-overtime/custom tagline dependencies.
- [x] Improve workday refresh busy state and genuine error feedback; keep storage/backup actual flows and explain replacement+restart in restore confirmation.
- [x] Run target runtime and existing settings suites; run `node scripts/run_all.js` with logs under this plan's workspace.

## Task 4: 文档、构建、检查与交付

**Files:** Modify `CHANGELOG.md`, `README.md`, `README.zh-CN.md`, `docs/CONVENTIONS.md`, and build-generated cache fingerprints.

**Interfaces:** No new interfaces; validates all prior tasks against spec and baseline.

- [x] Update unreleased user-facing notes and dual-language feature descriptions, document new frontend module count and settings behavior.
- [x] Run `build.bat test` in the supported common.bat/cmd environment; record pass/fail and counts, resolve regressions with evidence.
- [x] Run `build.bat debug` to regenerate/embed actual resources; verify cache fingerprints reach a stable build and review final diff for accidental changes.
- [x] Inspect actual front end with independent fixture data at 520×700. 2026-10-05 完成：frontend 复制至系统临时目录+前置 `__TAURI__` 桩（fixture 配置/DayStatus/备份列表）+本地 http 服务，浏览器视口精确 520×700、`configLoaded` 门闸正常，全程未触碰真实用户数据；目检后临时目录与服务已清理。不操作真实用户数据。
- [x] Request one final read-only whole-change review under requesting-code-review, fix material findings and run affected verification.
- [x] Mark this plan with final results and deliver code + doc paths and test evidence. Do not merge, push or publish unless separately requested.

## 执行结果

- 实施分支：`codex/settings-ui-redesign`；六组布局、逐项说明、预览、联动、错误反馈和串行保存已落地。
- `build.bat test`（联网环境）退出 0：fmt、clippy、cargo deny、254 项 Rust 测试、37 个前端测试脚本、29 + 8 项发布工具测试全部通过。
- `build.bat debug` 退出 0，产物 `bin/debug/niuma-timer.exe`；编译仅有增量缓存无法硬链接而改为复制的环境提示。
- 只读审查发现并修复：手动天数留空、隐藏原生 badInput、长期开启跨月覆盖、关闭功能时有效排队输入被回退。后续还补充工作日刷新响应的快照防护。
- 最终前端改动已再跑全套 37 个前端测试脚本，全部通过；`git diff --check` 通过。视觉验收因浏览器连接超时未完成，保留上述未勾选项。未合并、推送或发版。

## 视觉目检补录（2026-10-05）

- **方法**：frontend 复制到系统临时目录（不污染 `frontend/` 指纹），`stub.js` 注入到 core.js 之前提供 `__TAURI__`/`__TAURI_INTERNALS__` 桩（fixture 配置：月薪 10000、09:00–12:00/13:00–18:00、发薪日 10、加班费 25、餐补 15、白名单 1 项、保留期 90 天；DayStatus 带 monthly_workdays=22；5 份备份含 1 残缺），`python -m http.server` + 浏览器 `setViewportSize(520,700)`。
- **通过项**：六组内容与 520×700 布局（无横向溢出，页头吸附、滚动同步高亮）；月↔时薪切换显隐且发薪日两种模式可见；工作日手动模式自动填入已知 22 天并保存；计薪预览用真实工作日（¥56.82 时薪/¥454.55 日薪），非法时不伪造金额；加班关闭子参数禁用但数值保留；休息日费率空值显示沿用金额；监控关→白名单禁用、白名单关→编辑区收起且 chip 保留；自定义副标题就地预览+字数计数；账单风格双卡键盘方向键切换且 `aria-checked` 同步；发薪日 99 与作息倒挂均字段级标错（`aria-invalid`+红字）、页头错误态、toast、保存被阻止且磁盘快照保持旧值；修正后错误全部清除并恢复保存；Tab 焦点轮廓 2px 可见且与选中态可区分。
- **观察项（未修，低优）**：「改错→改回原值」后最后一次失焦因 `saveIfChanged` 去重跳过保存（配置已与磁盘一致），页头状态停留在「编辑后自动保存」而非回退为已保存——纯文案角落案例，保存数据正确；若要打磨可在 blur 等值时将状态降级为 saved/ready。
- 目检后临时目录、http 服务与浏览器标签页均已清理。仍未合并、推送或发版。
