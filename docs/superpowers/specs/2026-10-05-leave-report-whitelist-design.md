# 请假标记 + 月度战绩报告 + 白名单最近应用选择 设计

状态：2026-10-05 用户从优化清单中拍板 1/2/3 三项，按既有流程写 spec+plan 后实施。

## 目标与边界

三项互相独立又共享一处口径改动：

1. **日级请假/休假标记**：个人休假（年假/病假/事假/调休/其他）当天按休息日处理，修掉「请假日应赚停滞、摸鱼率爆表、账单出勤失真」的核心口径问题。
2. **月度战绩报告**：账单页新增「月报」tab，聚合一个月的实赚/出勤/休假/加班/摸鱼率/最拼最摸一天，可导出分享图（复用现有 canvas 导图链路）。
3. **白名单「从最近使用中添加」**：用手写应用名的痛点换成一键从真实使用记录中选择。

保留 Tauri 2、原生 JS、现有深色风格与 520×700 布局；不新增依赖；不改提醒规则本身（下班提醒经 state.rs 的 is_workday 覆盖后**自动**跳过休假天）；不动备份/还原流程结构。

## 一、请假标记

### 口径决策（v1 从简，文档化）

- **休假天一律按休息日处理**：不计应赚、不计出勤、不进摸鱼率、时间轴/走势隐藏。带薪假（年假计薪）作为未来可选细化，不在 v1。
- **月工作日分母剔除休假天**（自动模式）：月薪 ÷（本月工作日 − 休假数）= 剩余满勤日参考日薪。手动覆盖（workdays_override）模式下分母仍以手动值为准（day-level 休假标记与月分母手动覆盖互不覆盖彼此）。
- kind 仅作记录展示，不影响口径（v1 全部按休息日处理）。

### 数据与后端

- 新表 `day_override(date TEXT PRIMARY KEY, kind TEXT NOT NULL)`；登记 db.rs `CREATE_DAY_OVERRIDE` + `TABLE_DDL` + `BUSINESS_TABLES` + `TABLE_SINCE("1.9.1")`（旧备份 ≤1.9.0 缺此表合法放行，由既有 backup_should_have_table 机制保证）；maintain.rs `DATED_TABLES`（date 列）+ `TABLE_GROUPS/GROUP_ORDER`（等价性测试强制五处同登记）。
- 新模块 `src-tauri/src/dayoff.rs`：`today_kind(date)`（内存缓存按天，避免 get_status 每秒查库；set 时同步缓存）、`range_overrides(conn, start, end)`、`set(conn, date, kind)`。
- 新命令：`get_day_overrides(start, end) -> Vec<{date, kind}>`、`set_day_override(date, kind: Option<String>)`（None=删除）；invoke_handler + capabilities `allow-get-day-overrides` / `allow-set-day-override` 登记（test_commands 守卫校验）。
- **当日状态**：state.rs get_status 在节假日判定后叠加覆盖（is_workday=false），DayStatus 新增 `day_off_kind: Option<String>`；paused > day_off > 休息 > 下班的徽章优先级。
- **历史聚合**：weekbill.rs `is_workday_of` 叠加覆盖集（assemble 起止范围一次查库传入）；`monthly_workdays_of` 剔除当月休假天；所有 is_workday_of 调用点同步改签名。
- **提醒**：remind.rs:233 消费 st.is_workday，覆盖后自动生效，零改动。

### 前端

- 主界面品牌行加 ghost 小按钮「标记休假/取消休假」（仅日历工作日或已标记时可见；点击即存 kind="休假"）；状态徽章优先级新增休假档：**休假中 · 好好休息**。
- 设置页「薪资与作息」卡新增「休假记录」子区：日期输入 + 类型下拉 + 添加，列表展示近 90 天（日期 · 类型 · 删除）；空态说明影响口径。
- 工作日数行 hint 说明自动模式会剔除休假天。

## 二、月度战绩报告

- insights.js `BILL_TAB_KEYS` 增 `report`（排 bill 之后），pane/页名/分发同步；index.html 增 pane。
- 数据：复用 `get_bill("month", offset)` 的 PeriodBill + `get_day_overrides(月首, 月末)`，一次加载并行拉取；不需要新后端聚合命令。
- 页面内容：N 月战绩标题、实赚大字、出勤/休假/加班时长与加班费/摸鱼率/最拼一天/最摸一天、月度金句（按摸鱼率取档，未配月薪过滤提钱档，复用周账单金句规则）。
- 导出：pane 内「保存图片」按钮，canvas 750×1050 @2x 手绘（新 drawMonthlyReport，复用 drawReport 的工具函数与 export_image 落盘链路）；周/年跨度时显示提示 + 一键切到月跨度。

## 三、白名单从最近使用中添加

- 后端：`get_recent_app_names(days, limit) -> Vec<String>`（cmds_monitor.rs + app_usage.rs 查询：`SELECT app, SUM(seconds) FROM app_usage WHERE date >= ? GROUP BY app ORDER BY SUM(seconds) DESC LIMIT ?`，主键前缀扫描无新索引）；capabilities 登记。
- 前端：白名单输入行下方加「最近使用」pill 行（≤8 个，排除已在名单中的），点击即复用 addWhitelistItem 入列；名单增删后即时刷新；应用监控关闭/白名单收起时随容器禁用隐藏。样式复用现有 chip 体系。

## 验收

- runtime：dayoff 缓存与 set 同步、is_workday_of 覆盖（当天/历史）、monthly_workdays 剔除、备份含新表/旧备份缺表放行、get_recent_app_names 去重排序；前端：休假按钮显隐与徽章文案、月报渲染与导出门禁、最近应用 pill 过滤。
- `build.bat test` 全绿；构建指纹一并提交；双语 README/CHANGELOG/CONVENTIONS 同步；发版另听通知。

## 非目标

- 带薪假计薪口径、跨天调休互换（周六上班换周一休）、提醒规则变更、云同步、浅色主题。
