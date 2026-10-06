# 1.10.0 体验问题逐项修复计划

> **For agentic workers:** Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** 修复体验检查中已复现的五项问题，逐项交付说明、原因、方案和可重跑的验证方法。

**Architecture:** 沿用原生经典脚本结构，账单请求状态与数据绑定到跨度/偏移；渲染和导出共同保留未知状态。修正导航适用范围、暂停文案和画布坐标，不改变计薪规则，不新增依赖。

**Tech Stack:** Tauri 2、Rust、原生 HTML/CSS/JS、Node VM 回归检查。

**Spec:** `docs/2026-10-06-experience-audit.md` 的第一批五项；用户已明确要求逐个修复。

## Global Constraints

- 现有 CRLF 文件使用二进制读写保持换行。
- 不修改版本号、不推送、不发版；保持改动可审查。
- 本轮在当前授权工作目录内修改文件，不创建额外工作树，不提交 main。
- 每项先观察回归测试失败，再实现和观察通过；最后运行 build.bat test 及 build.bat debug 同步前端指纹。
- 第二批体验建议不扩入本轮。

## Review Focus

- 周/月/年快速切换、翻期、失败与重试：不能显示或导出错误周期。
- 空前台但存在加班/休假：保留有效数据，不给摸鱼评价。
- 时间线离开后回到周期页：周期偏好保留、导航恢复。
- 暂停与休假/休息日组合：明确暂停的是监控，不暗示工资冻结。
- 周/月/年柱图：位置、间隔、标签与有限坐标；长金额检查仍需真实字体目检。

## Tasks

- [x] 1. 在 scripts/test_experience_fixes.js 建立真实脚本隔离运行；先运行 stale 场景观察旧数据导出失败，再修 bill.js 的周期就绪状态、加载与失败显示、CSV/图片守卫以及重试绑定。运行 stale 与现有 bill_runtime/report_image/export_csv/bill_save。
- [x] 2. 运行 empty 场景观察零值和评价失败；修报告空态、未知摸鱼率及图片模型/绘制的证据门槛。验证仅有加班或休假仍能显示。
- [x] 3. 运行 navigation 场景观察时间线全局控件失败；给周期导航明确容器并在 setBillTabUI 切换可见性，增加程序入口守卫。验证回到周期页恢复。
- [x] 4. 运行 pause 场景观察文案冲突失败；统一首页、悬停卡、原生 tooltip 和说明为监控暂停，工资仍按作息推算。运行现有 guard/tagline/hover_card。
- [x] 5. 运行 chart 场景观察柱间隔与标题失败；修 drawReport 的槽位坐标与周期标题。验证 7/12/28/31 桶绘图坐标和未知状态绘制。
- [x] 6. 同步 README 双语、CHANGELOG、工程约定，运行完整门禁及 debug 编译，审查全部差异，记录每项验证结果与未验证边界。

## Execution record

- 已确认现状：当前普通 main 检出，仅审查文档为未跟踪文件；不移动或覆盖用户改动。
- 决策：用户已授权逐项修复，按当前清单在本会话连续执行；不再请求重复批准。
- 决策：保留现有工资推算规则，修正文案，不引入暂停扣薪。
- Task 1: complete — stale 回归先失败后通过，加载/失败/重试/账单图片/CSV 均覆盖。
- Task 2: complete — empty 回归先失败后通过，保留仅加班/休假，未知和真实零值分开。
- Task 3: complete — navigation 回归先失败后通过，时间线往返保留周期偏好。
- Task 4: complete — pause 回归先失败后通过，含休假/休息日及独立悬停卡渲染。
- Task 5: complete — chart 回归先失败后通过，7/12/28/31 桶及 null 摸鱼率绘制覆盖。
- Task 6: complete — build.bat test 最终退出 0（Rust 261、JS 38 组、Python 29+11+4）；build.bat debug 退出 0，前端指纹同步。fixture 双向契约补齐后再次运行全部 JS 38 组通过。
- Final review: 独立只读审查无阻塞性发现，相关 Node 检查通过。
- Final: deferred — 普通账单仅有手工加班时的既有空态与第二批体验建议不属于本轮五项修复；新报告已保留加班/休假证据。
- Final: 未执行真实 Windows 窗口、高 DPI、PNG 字体/剪贴板与实际暂停；人工验收方法见 docs/2026-10-06-experience-fixes-verification.md。
