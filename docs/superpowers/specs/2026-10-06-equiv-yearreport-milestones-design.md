# 摸鱼换算 + 年度报告 + 里程碑系统 设计

状态：2026-10-06 用户从发散清单拍板继续迭代（上轮 Top3：①摸鱼等价物换算 ②年度打工报告 ③里程碑系统；专注模式留作单独一批）。流程：spec/plan → 开发 → code review → 自验。

## 目标与边界

三个功能互相独立，共享「把已有数据翻译成情绪价值」的思路，**零新表、零新依赖**：

1. **摸鱼等价物换算**：把摸鱼成本换算成人话——「今天摸掉 ¥39 ≈ 2.6 杯奶茶」。主页烧钱行 + 月报/账单图各一处。
2. **年度报告**：账单页报告 tab 从「仅月」泛化为「月 + 年」：年跨度渲染年度战绩卡（总赚/出勤/加班/摸鱼率/最拼一天/最拼月/休假天数/年度金句），支持存图；去掉「年跨度禁存图」的旧限制。
3. **里程碑系统**：用这台应用以来的累计成就（加班时长/键盘敲击/键鼠移动/专注时长/相伴天数），三档阶梯 + 进度条，展示在报告 tab 底部。

保留 Tauri 2、原生 JS、现有风格；不改任何计算口径；不动备份/还原结构。

## 一、摸鱼等价物换算

- **单位预设**（前端常量表 core.js）：杯奶茶 ¥15 / 杯咖啡 ¥30 / 顿外卖 ¥25 / 张电影票 ¥40 / 自定义金额 / 关闭。价格随预设固定；「自定义」用配置的金额。
- **配置**：`slack_equiv_unit: String`（默认 "milktea"）+ `slack_equiv_price: f64`（默认 15.0，仅自定义用）；serde default 兼容旧配置（老用户默认开奶茶档）。设置 → 外观新增「摸鱼换算」下拉 + 自定义时显示金额输入行（`.hidden` 显隐），change 即存。
- **展示**：换算值 = floor(摸鱼成本 ÷ 单价 × 10) / 10；成本 > 0 且单位非关闭时显示「≈ N.x 杯奶茶」，< 0.1 显示「<0.1」。主页烧钱行（renderSlackBurn）追加；报告卡与报告图各加一行（drawReport 行数组按模型字段条件插入）。
- **纯函数**：`fmtSlackEquiv(cost, unitKey, customPrice)` 收口 core.js（与 fmtMoney 同族），三处消费。

## 二、年度报告（报告 tab 泛化）

- **数据**：复用 `get_bill("year", offset)`（后端年聚合已存在：12 个月桶 `month_bucket`，`hardest/slackiest` 仍为天粒度）+ `get_day_overrides(年首, 年末)`。休假天数 = 范围内标记数。
- **渲染**：`loadMonthlyReport → loadReport`、`paintMonthlyReport → paintReport` 泛化：标题「N 月战绩 / N 年战绩」；行 = 出勤 / 休假（升序日期串）/ 加班 / 摸鱼率 / 最拼一天（后端 hardest）/ 最摸一天；**年跨度额外加「最拼月」**（前端从 12 桶里取 ot_total 最大且有记录的月，桶粒度后端不算）。周跨度维持引导态。
- **金句**：`spanQuipText` 扩展 year 分支（本周→今年）。
- **导图**：`drawReport` 标题支持「年账单」；12 个月桶标签 ≤12 自动显示（现有逻辑）；「存为图片」对年跨度**解禁**（`setBillSpanUI` 的 imgBtn.disabled 与 `saveBillImage` 的年拦截一并移除，文件名 niuma-年报-*.png）；对应守卫 `test_report_image` 的「年跨度双重拦截」断言改为「年跨度已解禁」——原设计「年图价值低」被年报功能推翻，属守卫本义演进。
- **页名**：翻页器页名随跨度动态（月跨度「月报」/年跨度「年报」），圆点 title「月报 / 年报」。

## 三、里程碑系统

- **后端**：新模块 `milestones.rs`（聚合既有表，零新表）+ 命令 `get_milestones`（cmds_core，async）：
  - `ot_hours`（Σ ot_records.valid_hours）、`ot_fee`（Σ total）
  - `keystrokes`（Σ act_hourly.keys）、`distance_px`（Σ act_hourly.pixels）
  - `focus_minutes`（Σ focus_sessions.minutes）、`active_days`（COUNT DISTINCT date）
  - `first_date`（MIN date，展示「相伴 N 天」用前端以 first_date 起算？——用 active_days 直接表达，first_date 仅展示）
- **阶梯**（前端常量，后端只出原始量）：加班 100h/500h/1000h；敲键 100 万/500 万/1000 万；移动 10km/50km/100km（96dpi 换算，复用 core.js fmtDist 口径）；专注 50h/200h/500h；相伴 30/100/365 天。
- **展示**：报告 tab 底部「里程碑」区（与跨度无关，进报告页即拉）：每行 = 名称 + 当前值 + 下一档进度条 + 「已达成 x/3」；全部达成显示满档徽标。
- **非目标**：达成时的托盘通知/持久化已读状态（v1 纯展示，留作迭代）；「赚满 ¥10 万」类里程碑（口径复杂，不做）。

## 验收

- 单元：config serde 默认、fmtSlackEquiv 档位、milestones SQL 聚合（in-memory 播种）、spanQuipText year 措辞、buildReportModel 透传。
- runtime 守卫同步：test_report_image（年解禁 + 三参签名）、CONVENTIONS 计数（.rs 34→35、命令 42→43/权限 50→51、测试数）。
- 桩环境目检：烧钱行换算、外观设置换算单位切换、年跨度年报卡+存图、里程碑进度条。
- `build.bat test` 全绿；CHANGELOG [未发布] + 双语 README；提交不推送。
