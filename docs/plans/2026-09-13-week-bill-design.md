# 本周打工账单：小票 / 仪表盘双风格周聚合视图

- 日期：2026-09-13
- 状态：已与用户逐节确认（定位=晒单+复盘融合；范围=本周起可翻历史周；入口=侧栏第四 tab；两风格都做、设置可切、默认小票）
- 依据：释放已在攒但跨天即丢失的历史数据价值；「赚钱 + 摸鱼」软件身份的周末账单表达

## 目标

1. 第一次把库里跨天数据汇总成一张「本周打工账单」，可复盘、可截图
2. 两种视觉风格共存：**小票风（默认，晒单情绪）** 与 **仪表盘风（数据复盘）**，设置页切换
3. 口径集中在 Rust 一处聚合，前端只负责渲染；复用现有工资 / 加班 / 分类 / 工作日判定逻辑

## 入口与交互

- 侧栏在「主页」与「明细」之间插入第四个 tab：主页 · **账单** · 明细 · 设置
  （`data-nav="viewBill"`，Segoe Fluent 收据/账单字形）
- 账单页顶部周导航：`‹ 第37周 · 09.08–09.14 ›`，状态 `weekOffset`（0=本周）
  往前可翻到有数据的历史周；往后到本周封顶（不可看未来）
- 懒渲染契约新增一条：进入 viewBill 时按当前 weekOffset 拉取渲染；账单是历史聚合，
  **不接 tick 轮询**；返回页面（onShow）时重读设置里的 bill_style 并重渲染
- 导航既有三条契约（懒渲染守卫、返回主页 resetHistDates、离开设置 saveIfChanged）不变

## 两种风格（同一 WeekBill 数据，两套渲染）

- **小票风 receipt（默认）**：居中大金额「本周总入账」+ 周损味金句卡 + 虚线小票行
  （应赚工资 / 加班费 / 摸鱼成本 / 出勤工时）+ 7 日金条（周末标「休息」）+ 右下斜红章
- **仪表盘风 dashboard**：总入账 KPI 卡（带环比箭头）+ 2×2 指标格（工资 / 加班 / 摸鱼 / 出勤工时）
  + 最累 / 最摸一行 + 每日入账(金)/摸鱼(红)双柱图 + 键鼠强度脚注；不显示金句

## 设置项

- `Config` 新增 `bill_style: String`，取值 `"receipt"` / `"dashboard"`，默认 `"receipt"`
  （config.rs 加字段 + serde default，旧配置文件缺字段自动回退默认）
- 设置页「界面」区新增「账单风格」分段选择，走现有 saveIfChanged 自动保存
- 账单 onShow 重读该字段，切风格后返回账单即生效

## 数据与 IPC

新增 **1 个** 命令：`get_week_bill(week_offset: i64) -> WeekBill`，main.rs 注册；
聚合逻辑放新文件 `src-tauri/src/weekbill.rs`。后端由 today 与 offset 对齐到周一，
前端只传偏移，避免日期口径漂移。

```
WeekBill {
  week_start, week_end, week_no: String,
  is_current_week: bool,
  total_income, base_salary, ot_fee, slack_cost: f64,
  prev_total: f64, delta_pct: Option<f64>,
  work_days: Option<i64>, work_hours: f64,
  keys_total, clicks_total: i64,
  hardest: Option<{date, ot_hours}>,
  slackiest: Option<{date, rate}>,
  days: [DayBill; 7],
}
DayBill { date, weekday: String, is_workday: bool, salary, ot_total,
          slack_seconds, act_events: i64, keys, clicks: i64,
          slack_rate: f64, has_record: bool }
```

- `act_events` = 当天 act_hourly 的 total_events（moves+left+dbl+right+wheel+mid+xbtn+keys），
  用于出勤判定与柱图；`has_record` = `act_events>0` 或当天 app_usage 秒数>0（有任意监控证据）

### 口径

1. **应赚工资 base_salary**：库内无打卡工时——
   - 历史工作日按**满勤** = `月薪 ÷ 当月工作日数`；工作日数降级链
     `effective_workdays_override → holiday.month_workdays → weekday_count`
   - 跨月的一周，每一天按其所属月份分母单独算
   - 本周内的**今天**用实时 `compute().earned`；本周内**未来**工作日计 0（不预支）；休息日 0
2. **加班费 ot_fee**：新增 `SELECT ... FROM ot_records WHERE date BETWEEN ? AND ?`，
   直接 `SUM(total)`（fee+meal 已落盘），不按费率重算，跨月天然正确
3. **摸鱼成本 slack_cost**：区间 `SELECT app, SUM(seconds) ... GROUP BY app`，
   复用 `app_usage::category_of` + `fold_categories` 得摸鱼秒；**按天折算**
   （每天摸鱼秒 × 当天时薪后求和），不用周平均时薪（跨月时薪可能不同）；
   图标装配仍在 DB 锁外（ABBA 死锁规避，沿用本模块约束）
4. **总入账 total_income = base_salary + ot_fee**；摸鱼成本只作「烧钱」展示，**不做减法**
   （延续「不做净到手」的既有决定）
5. **出勤天数 / 工时**：work_days = 本周内**有活动或应用记录**的工作日数（实际出勤证据，
   非应出勤）；work_hours = 出勤工作日 × `calc::daily_hours` + 加班 valid_hours，
   为排班口径估算，UI 用「在岗约 43.5h」弱化措辞，不声称真实打卡
6. **键鼠强度**：`act_hourly` 区间求和。keys = `SUM(keys)`；
   点击沿用活动页口径 = 左键`(left − 2×dbl)` + 双击`dbl` + 右键`right` + 中键`mid` + 侧键`xbtn`
   （双击同时计入 left，故先减 2×dbl；滚轮 wheel 不计「点击」）
7. **最累 / 最摸**：组装 DayBill 时取 max——最累=ot_hours 最大；
   最摸=有记录日中 slack_rate 最高（slack 秒 ÷ 四类前台总秒，沿用首页 C1 口径）；
   无记录不入选，缺数据给 null
8. **环比**：同函数聚合上一周（offset+1）total_income，`(本周-上周)/上周`；
   上周为 0 / 无数据 → delta_pct=null（前端不显示箭头，不出现 ∞）
9. **跨年周**：工作日判定按天的年份取缓存——AppState 当前年缓存 →
   `holiday::builtin_cache(该年)` → 周几规则降级；不触发网络刷新

## 边界与空状态

| 情况 | 处理 |
|---|---|
| 整周零数据 | 账单区空态「这周还没有打工记录」+ 小马图标位，不排一排 ¥0.00；仍可翻周 |
| 休息日 | 不计工资/摸鱼；小票金条不渲染标「休息」；仪表盘柱位留空 |
| 本周未来工作日 | 工资 0，灰显「未到」 |
| 未配月薪（时薪 0） | 工资、摸鱼成本行隐藏；总入账只体现加班费；金句走纯摸鱼率版（不提钱） |
| 监控开关关闭 | 摸鱼行/最摸隐藏、键鼠显示「—」；活动与应用监控都关时 work_days 显示「—」 |
| 环比无上周基数 | delta_pct=null，不显示箭头 |
| IPC/SQL 失败 | 沿用 with_db 的 Err(String)；前端 catch 轻提示「账单加载失败」，不白屏 |

### 周损味金句（小票风，按周摸鱼率）

| 周摸鱼率 | 金句 |
|---|---|
| <10% | 本周天选牛马，老板的战略合作伙伴 |
| 10–25% | 摸得克制，装得敬业 |
| 25–40% | 将近三分之一的班，上给了手机 |
| ≥40% | 本周工资建议原路退回 |

无任何应用记录则不显示金句（数据不足不评判）。

## 涉及文件

| 文件 | 改动 |
|---|---|
| src-tauri/src/weekbill.rs | 新增：区间 SQL + WeekBill 组装 + 纯聚合单测 |
| src-tauri/src/main.rs | mod 注册 + `get_week_bill` 命令 |
| src-tauri/src/config.rs | `bill_style` 字段 + serde 默认 receipt |
| frontend/index.html | rail 第四 tab；viewBill 骨架（周导航 + 两风格容器）；设置页风格分段 |
| frontend/styles.css | 小票风（rcp-*）/ 仪表盘风（kpi/mini/bars）/ 周导航 / 空态样式 |
| frontend/app.js | weekOffset 状态与翻页、invoke get_week_bill、双模板渲染、守卫分支、设置绑定 |
| scripts/test_week_bill.js | 新增静态断言 |
| CHANGELOG.md | 未发布区补记录 |

## 不做的事

- 不改任何表结构、不改既有命令签名
- 不做本月 / 自由区间（只做按周）
- 不做截图导出、不做账单相关通知
- 摸鱼成本不从总入账里减（不做净到手）
- work_hours 不伪装成真实打卡工时（排班估算 + 弱化措辞）

## 测试

- Rust：`weekbill.rs` 内 `#[cfg(test)]`——周一日期对齐、跨月周工资分母、
  今日实时 vs 历史满勤 vs 未来 0、摸鱼按天折算、环比除零 null、
  最累/最摸选取且排除无记录日、休息日跳过
- 前端 `scripts/test_week_bill.js`：第四 tab 与 viewBill 骨架、weekOffset 本周封顶、
  双模板按 bill_style 切换、三守卫（休息日/未来/时薪 0）、空态分支、
  设置分段与 Config 字段、get_week_bill 命令注册、`$()` 兜底
- 回归：全量 `scripts/test_*.js` + `cargo test` + `build.bat debug`（缓存戳自动同步）
