# 摸鱼换算 + 年度报告 + 里程碑系统 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 落地用户拍板的三个情绪价值功能：摸鱼等价物换算、年度报告（报告 tab 泛化月+年）、里程碑系统。零新表零新依赖。

**Spec:** `docs/superpowers/specs/2026-10-06-equiv-yearreport-milestones-design.md`

## Global Constraints

- 不改计算口径；不新增数据表；命令三件套（command/handler/capabilities，build.rs 自动补权限）。
- 守卫字面量随签名演进同步（test_report_image 等）。
- 提交不推送；发版另听通知。

## Task 1: 摸鱼等价物换算

**Files:** `frontend/js/core.js`, `frontend/js/monitor.js`, `frontend/js/bill.js`, `frontend/index.html`, `frontend/js/settings.js`（readCfg/绑定）, `frontend/styles.css`, `src-tauri/src/config.rs`。

- [x] config.rs：`slack_equiv_unit`（默认 "milktea"）+ `slack_equiv_price`（默认 15.0）+ serde default；单测默认值（序列化-删字段-反序列化验证旧配置兜底）。
- [x] core.js：SLACK_UNITS 表 + `fmtSlackEquiv(cost, unitKey, customPrice)` 纯函数。
- [x] 设置 → 外观：换算单位下拉 + 自定义金额行（hidden 显隐），readCfg 采集 + change 即存；JS 守卫（test_report_image 补换算透传断言）。
- [x] monitor.js renderSlackBurn 追加换算；bill.js 报告卡行内联 + buildReportModel 三参透传 + drawReport 摸鱼率行内联（避免固定版式被行数挤爆）；saveBillImage/saveReportImage 传入。
- [x] cargo test + JS 套件绿。

## Task 2: 年度报告（报告 tab 泛化）

**Files:** `frontend/js/insights.js`, `frontend/js/bill.js`, `scripts/test_report_image.js`。

- [x] spanQuipText 加 year 措辞；drawReport 标题「年账单」；年解禁（移除 setBillSpanUI imgBtn.disabled + saveBillImage 年拦截）；守卫断言改写（「年跨度双重拦截」→「年跨度已解禁」+ 标题随周/月/年）。
- [x] loadMonthlyReport→loadReport / paintMonthlyReport→paintReport 泛化（week 引导、month/year 渲染；年加「最拼月」客户端从 12 桶计算）；页名随跨度（月报/年报，boot.js 切跨度补 setBillTabUI 同步）；buildReportModel 传 leaveDays+slackEquiv。
- [x] 月报守卫与 runtime 测试同步（test_bill_save 补 setBillTabUI/curBillTab 桩）；套件绿。

## Task 3: 里程碑系统

**Files:** Create `src-tauri/src/milestones.rs`; modify `cmds_bill.rs`, `main.rs`, `frontend/index.html`, `frontend/js/bill.js`, `frontend/styles.css`, `docs/CONVENTIONS.md`。

- [x] milestones.rs：`assemble(conn) -> Milestones`（六项聚合）+ in-memory 单测 2 条；命令 get_milestones 三件套（capabilities 由 build.rs 自动补）。
- [x] 前端：报告 tab 底部里程碑区（MILESTONE_LADDERS 常量 + 进度条渲染；专注 fmt 用 fmtDurCN 避免「49 分钟→1 h」穿帮）；loadReport 进页即拉。
- [x] CONVENTIONS 计数更新（.rs 35、测试 263、命令 43/权限 51）；cargo test + JS 套件绿。

## Task 4: 全量验证与交付

- [x] `build.bat test` 全绿；CHANGELOG [未发布] + 双语 README。
- [x] code review（只读全量 diff → 修 3 处：CHANGELOG 重复「### 新增」段合并、insights.js `};let` 挤行还原、里程碑专注 fmt 穿帮）。
- [x] 桩环境目检。**抓到并修复 1 个发布级事故**：settings.js 顶层 `addEventListener("change", renderSlackBurn)` 在解析期直接引用后加载的 monitor.js 函数 → ReferenceError 截断整个 settings.js → toastTimer 进 TDZ → 设置保存链路全灭；修复为箭头函数延迟引用并注释事故原因。目检通过项：主页烧钱行「¥39.06 摸鱼率 21% · ≈2.6 杯奶茶」、外观单位切换（custom 显价格行/off 去换算）、月报卡换算内联、年报卡（2026 年战绩/最拼月 3 月/休假聚合/存图链路）、里程碑五档进度（专注「76小时40分」）。
- [x] 提交（不推送）；汇总已验/待人工验清单。

## 执行结果

- 三功能全部落地：`build.bat test` 全绿（Rust 263 测试、JS 37 套件），零新表零新依赖。
- 桩环境目检发现 1 个发布级事故（settings.js 顶层引用后加载函数导致解析中断）并修复——该类错误的教训已写入代码注释：**顶层传函数引用必须包箭头函数延迟解析**。
- 未推送、未发版。
