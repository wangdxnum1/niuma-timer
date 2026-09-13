# 周账单功能实现计划（week-bill-plan）

> 设计权威来源：[2026-09-13-week-bill-design.md](./2026-09-13-week-bill-design.md)（已提交 271a845）
> 本计划所有签名/锚点均已在当前代码实读核实；执行顺序 A → B → C → D → E。

---

## Task A：新建 `src-tauri/src/weekbill.rs`

### A1. 数据结构（全部 `#[derive(Serialize)]`，字段 snake_case 直达前端）

```rust
#[derive(Debug, Serialize)]
pub struct DayBill {
    pub date: String,        // "2026-09-08"
    pub weekday: String,     // "周一"
    pub is_workday: bool,    // 法定口径（节假日表/周几兜底）
    pub salary: f64,         // 应赚工资：休息日 0；今天 = calc::compute().earned；过去工作日 = 满勤
    pub ot_total: f64,       // 当日加班费合计（ overtime 流水 SUM(total)）
    pub slack_seconds: i64,  // 摸鱼秒（分类=摸鱼 的应用时长和）
    pub act_events: i64,     // 事件总数口径 = moves+left+dbl+right+wheel+mid+xbtn+keys
    pub keys: i64,
    pub clicks: i64,         // left+dbl+right+mid+xbtn（滚轮不计）
    pub slack_rate: f64,     // 当日摸鱼率 0–1；无工作分类记录时 0
    pub has_record: bool,    // 当日有任何活动/加班/应用记录
}

#[derive(Debug, Serialize)]
pub struct HardestDay { pub date: String, pub weekday: String, pub ot_hours: f64 }

#[derive(Debug, Serialize)]
pub struct SlackiestDay { pub date: String, pub weekday: String, pub rate: f64 }

#[derive(Debug, Serialize)]
pub struct WeekBill {
    pub week_start: String,
    pub week_end: String,
    pub week_no: String,          // "第 37 周"
    pub is_current_week: bool,
    pub total_income: f64,        // base + ot_total − slack_cost
    pub base_salary: f64,         // 7 天 salary 之和（含今日实时 earned）
    pub ot_fee: f64,              // 7 天 ot_total 之和
    pub slack_cost: f64,          // 摸鱼成本 = Σ(slack_seconds/3600 × 当日时薪)
    pub prev_total: f64,
    pub delta_pct: Option<f64>,   // 单位百分点 ((cur−prev)/prev×100)，prev<=0 → None
    pub work_days: Option<i64>,   // 有打卡记录的工作日数；监控全关 → null
    pub work_hours: f64,
    pub keys_total: i64,
    pub clicks_total: i64,
    pub slack_rate: f64,          // 周摸鱼率（分子/分母均只含工作日）
    pub front_seconds: i64,       // 工作日四类前台总秒（前端金句显示门槛）
    pub hardest: Option<HardestDay>,
    pub slackiest: Option<SlackiestDay>,
    pub days: Vec<DayBill>,
}
```

### A2. 纯函数（供单测与聚合复用）

```rust
use chrono::{Datelike, Duration, NaiveDate, Weekday};

pub fn week_start_of(d: NaiveDate) -> NaiveDate {
    d - Duration::days(d.weekday().num_days_from_monday() as i64)
}

/// 法定工作日口径：当天年份若非当前缓存年份，取内置表（owned，解决生命周期）；
/// 无表退回周一至五；holiday::builtin_cache(year) 对晚/早年份可能为 None。
fn is_workday_of(date: NaiveDate, cur: &holiday::HolidayCache) -> bool {
    let local: Option<holiday::HolidayCache> = if cur.year == date.year() {
        None
    } else {
        holiday::builtin_cache(date.year())
    };
    let hol = local.as_ref().unwrap_or(cur);
    match hol.is_workday(date) {
        Some(v) => v,
        None => date.weekday() != Weekday::Sat && date.weekday() != Weekday::Sun,
    }
}

/// 当月工作日分母：override > 节假日表 > weekday_count（同样跨年取内置表）
fn monthly_workdays_of(year: i32, month: u32, cfg: &config::Config, cur: &holiday::HolidayCache) -> u32 {
    if let Some(v) = config::effective_workdays_override(cfg, year, month) {
        return v;
    }
    let local: Option<holiday::HolidayCache> = if cur.year == year { None } else { holiday::builtin_cache(year) };
    let hol = local.as_ref().unwrap_or(cur);
    hol.month_workdays(year, month).unwrap_or_else(|| holiday::weekday_count(year, month))
}
```

### A3. 聚合入口

```rust
pub struct WeekInput<'a> {
    pub cfg: &'a config::Config,
    pub cur_hol: &'a holiday::HolidayCache,
    pub week_start: NaiveDate,
    pub today: NaiveDate,
    pub today_earned: f64,   // calc::compute() 的 earned，main.rs 锁外算好传入
}

pub fn assemble(input: &WeekInput, conn: &Connection) -> rusqlite::Result<WeekBill>
pub fn week_bill(cfg: &config::Config, cur_hol: &holiday::HolidayCache, week_offset: i64) -> Result<WeekBill, String>
pub fn delta_pct(cur: f64, prev: f64) -> Option<f64> {
    if prev <= 0.0 { None } else { Some((cur - prev) / prev * 100.0) }
}
```

`week_bill` 流程：
1. `let off = week_offset.max(0);`（未来封顶本周）
2. `let today = Local::now().date_naive();` `let ws = week_start_of(today - Duration::weeks(off));`
3. `let today_earned = { /* calc::compute(cfg, is_workday_of(today,…), monthly_workdays_of(today), now) */ }` —— 锁外由调用方传入更佳：本函数内部直接调用 `calc::compute`（cfg 为快照引用，无锁需求）。
4. 第一次 `assemble`（带 conn）取 cur；prev 周 = `ws − 7d`，第二次 assemble 只为取 `total_income`。
5. `is_current_week = off == 0`。

### A4. SQL（全部在 `assemble` 内，`with_db` 外部已拿连接）

```sql
-- 加班：overtime 表
SELECT date, SUM(total), SUM(valid_hours) FROM overtime
 WHERE date >= ?1 AND date < ?2 GROUP BY date;

-- 活动计数：act_hourly
SELECT date,
       SUM(moves)+SUM(left_clicks)+SUM(dbl_clicks)+SUM(right_clicks)
       +SUM(wheel)+SUM(mid_clicks)+SUM(xbtn)+SUM(keys) AS total_events,
       SUM(keys) AS keys,
       SUM(left_clicks)+SUM(dbl_clicks)+SUM(right_clicks)+SUM(mid_clicks)+SUM(xbtn) AS clicks
  FROM act_hourly WHERE date >= ?1 AND date < ?2 GROUP BY date;

-- 应用分类秒：app_usage（每行含 duration 秒）
SELECT date, app, SUM(duration) FROM app_usage
 WHERE date >= ?1 AND date < ?2 GROUP BY date, app;
-- Rust 侧 category_of(app) 归类（config::app_categories 精确名 > 关键词 > "other"）
```

字段名以 db.rs DDL 常量为准（CREATE_OT_RECORDS：date/total/valid_hours…；CREATE_ACT_HOURLY：date/moves/left_clicks/dbl_clicks/right_clicks/wheel/mid_clicks/xbtn/keys…；CREATE_APP_USAGE：date/app/duration…）。

### A5. 工资口径（assemble 内）

- `full_day_salary = cfg.monthly_salary / monthly_workdays_of(该日年月)` —— **每天用自己所属月的分母**（跨月周：8 月天用 8 月分母，9 月天用 9 月分母）
- 休息日 salary=0、ot 有流水照记、slack_rate 记但不算进周率分母
- 今天：salary = `input.today_earned`（实时）；过去工作日 = 满勤
- 摸鱼成本当日时薪 = salary / daily_hours（`calc::daily_hours(cfg)`）；salary=0 天成本 0

### A6. `#[cfg(test)]` 单测

- `mem_conn()`：`Connection::open_in_memory()` + `execute_batch(CREATE_OT_RECORDS; CREATE_ACT_HOURLY; CREATE_APP_USAGE)`（db.rs 常量为 `pub(crate)`，可直接用）
- `hol_2026()`：`holiday::HolidayCache { year: 2026, ..Default::default() }`（空表 → 周几兜底）
- 用例：
  1. `week_start_of`：周三回周一、周一原地、周日回 6 天前
  2. `delta_pct`：prev=0 → None；正常增长/下降
  3. 跨月满勤：2026-08-31..09-06 周（8 月分母 21、9 月分母 22，晚 8 月无假期），月薪 22000 → Mon(8/31) salary = 22000/21，Tue(9/1) = 22000/22
  4. 休息日 salary=0；加班费照记
  5. act 聚合：插入 moves=100/left=10/dbl=2/right=3/wheel=50/mid=1/xbtn=0/keys=200 → act_events=316、clicks=16
  6. app 分类：工作 3600s + 摸鱼 1200s → slack_rate = 1200/4800 = 0.25
  7. work_days：有活动记录的工作日数

## Task B：main.rs + config.rs 接线

### B1. config.rs
- 字段区（retention_days 之前或其后）：

```rust
    /// 账单展示风格：receipt 小票 / dashboard 仪表盘
    #[serde(default = "default_bill_style")]
    pub bill_style: String,
```

- default 函数：`fn default_bill_style() -> String { "receipt".into() }`
- Default impl（L169 retention_days 后）：`bill_style: "receipt".into(),`

### B2. main.rs
- `mod weekbill;`（插 L16 `mod tray;` 后、`mod win;` 前——保持字母序实为 tray < weekbill < win）
- 命令（放在 get_app_usage_summary 附近）：

```rust
#[tauri::command(async)]
fn get_week_bill(
    state: State<'_, AppState>,
    week_offset: i64,
) -> Result<weekbill::WeekBill, String> {
    let (cfg, hol) = {
        let cfg = state.config.lock().unwrap().clone();
        let hol = state.holiday.lock().unwrap().clone();
        (cfg, hol)
    };
    weekbill::week_bill(&cfg, &hol, week_offset)
}
```

- generate_handler! 列表加 `get_week_bill,`（get_app_usage_summary 后）
- **capabilities 不手改**：build.rs sync_capabilities 自动生成 allow-get-week-bill（构建后验证）

## Task C：测试先行（FAIL → GREEN）

### C1. 新建 `scripts/test_week_bill.js`
结构仿 test_slacking.js：readFileSync 三件套（weekbill.rs / app.js / index.html）→ eq/has 断言 → `$()` 引用兜底扫描（weekbill.rs 里的 `$("` 视为前端串台）→ `process.exit(fail ? 1 : 0)`。断言覆盖：
- weekbill.rs：WeekBill/DayBill/HardestDay/SlackiestDay 结构、week_start_of、delta_pct、assemble、week_bill、CREATE_OT_RECORDS 引用、cfg(test)
- app.js：get_week_bill invoke、weekOffset、WEEK_BILL_QUIPS 四档原文、paintWeekBill/paintReceipt/paintDash/readBillStyle/setBillStyleUI、billNextWeek.disabled
- index.html：viewBill、billPrevWeek/billWeekLabel/billNextWeek、billReceipt、billDash、data-bill="receipt"、data-bill="dashboard"
- main.rs：get_week_bill 命令与 generate_handler 登记

### C2. 更新 `scripts/test_topnav.js` 三处
1. `eq("视图数量（.app）", viewIds.length, 6)` → `7`
2. railNavs 期望 `["viewMain","detail","viewSettings"]` → `["viewMain","viewBill","detail","viewSettings"]`
3. 标签数组 `[">主页<", ">明细<", ">设置<"]` → 插 `">账单<"`

先跑：`node scripts/test_week_bill.js`（应 FAIL）→ 全部实现后转 GREEN。

## Task D：前端三件套

### D1. index.html
1. rail（L48-59）：viewMain 与 detail 之间插：

```html
<div class="rail-item" data-nav="viewBill" title="周账单"><span class="rail-ico">&#xE8AF;</span><span class="rail-label">账单</span></div>
```

2. viewMain 闭合（L202 `</div>`）后、viewSettings 注释前插整个 viewBill 块：

```html
<!-- ====================== 周账单 ====================== -->
<div id="viewBill" class="view">
  <div class="month-nav">
    <button id="billPrevWeek" class="mn-btn">‹ 上一周</button>
    <span id="billWeekLabel" class="mn-label">—</span>
    <button id="billNextWeek" class="mn-btn">下一周 ›</button>
  </div>

  <div id="billEmpty" class="bill-empty" style="display:none">
    <div class="bill-empty-ico">&#xE8B6;</div>
    <div class="bill-empty-txt">这一周还没有任何记录</div>
  </div>

  <div id="billReceipt" class="receipt" style="display:none">
    <div class="rcp-head">
      <div class="rcp-title">本周打工账单</div>
      <div class="rcp-no" id="rcpNo">NO.0000</div>
    </div>
    <div class="rcp-amount" id="rcpAmount">¥0.00</div>
    <div class="rcp-quote" id="rcpQuote">—</div>
    <div class="rcp-lines" id="rcpLines"></div>
    <div class="rcp-days" id="rcpDays"></div>
    <div class="rcp-footnote" id="rcpFootNote">—</div>
    <div class="rcp-stamp">已验讫</div>
  </div>

  <div id="billDash" class="bill-dash" style="display:none">
    <div class="kpi-row">
      <div class="kpi">
        <div class="kpi-val" id="kpiAmount">¥0.00</div>
        <div class="kpi-sub" id="kpiDelta"></div>
      </div>
    </div>
    <div class="mini-grid">
      <div class="mini"><span class="mini-k">基本工资</span><span class="mini-v" id="miniSalary">—</span></div>
      <div class="mini"><span class="mini-k">加班费</span><span class="mini-v" id="miniOt">—</span></div>
      <div class="mini"><span class="mini-k">摸鱼成本</span><span class="mini-v" id="miniSlack">—</span></div>
      <div class="mini"><span class="mini-k">出勤</span><span class="mini-v" id="miniHours">—</span></div>
    </div>
    <div class="dash-extreme" id="dashExtreme"></div>
    <div class="dash-bars" id="dashBars"></div>
    <div class="dash-foot" id="dashFoot"></div>
  </div>
</div>
```

3. 设置卡基础分段（workdaysInfo 后、`</section>` 前）插账单风格：

```html
<div class="form-row">
  <span class="lbl">账单风格</span>
  <div class="mon-seg" id="billStyleSeg">
    <button class="mon-seg-item" data-bill="receipt">小票</button>
    <button class="mon-seg-item" data-bill="dashboard">仪表盘</button>
  </div>
</div>
```

4. 缓存戳 v=925f1fe7 → 新戳（build.bat 同步，两处：styles.css 与 app.js）

### D2. app.js（账单块插 hist 导航绑定后、rail 注释前）

```js
// ============ 周账单 ============
const WEEK_BILL_QUIPS = [
  { min: 0,    text: "本周天选牛马，老板的战略合作伙伴" },
  { min: 10,   text: "摸得克制，装得敬业" },
  { min: 25,   text: "将近三分之一的班，上给了手机" },
  { min: 40,   text: "本周工资建议原路退回" },
];
function weekBillQuip(ratePct) {
  let q = WEEK_BILL_QUIPS[0].text;
  for (const t of WEEK_BILL_QUIPS) if (ratePct >= t.min) q = t.text;
  return q;
}
let weekOffset = 0;
let billData = null;
const fmtMoney = (v) => "¥" + Number(v || 0).toFixed(2);
function moneyConfigured() { return parseFloat($("monthly_salary").value) > 0; }
function readBillStyle() { return window.__billStyle || "receipt"; }
function setBillStyleUI(style) {
  window.__billStyle = style || "receipt";
  const seg = $("billStyleSeg");
  if (seg) seg.querySelectorAll(".mon-seg-item").forEach(b =>
    b.classList.toggle("active", b.dataset.bill === window.__billStyle));
}
async function loadWeekBill() {
  try {
    billData = await invoke("get_week_bill", { weekOffset });
    paintWeekBill();
  } catch (e) { console.error("get_week_bill", e); }
}
async function shiftWeek(delta) {
  const next = weekOffset + delta;
  if (next < 0) return;
  weekOffset = next;
  await loadWeekBill();
}
function paintWeekBill() {
  if (curView !== "viewBill") return;   // 懒渲染守卫
  const bill = billData;
  if (!bill) return;
  const label = bill.week_no + " · " + bill.week_start.slice(5).replace("-", ".")
    + "–" + bill.week_end.slice(5).replace("-", ".");
  $("billWeekLabel").textContent = label;
  $("billNextWeek").disabled = bill.is_current_week;
  const empty = !bill.days.some(d => d.has_record);
  $("billEmpty").style.display = empty ? "" : "none";
  $("billReceipt").style.display = (!empty && readBillStyle() === "receipt") ? "" : "none";
  $("billDash").style.display = (!empty && readBillStyle() === "dashboard") ? "" : "none";
  if (empty) return;
  if (readBillStyle() === "receipt") paintReceipt(bill); else paintDash(bill);
}
function paintReceipt(bill) { /* 金句条件 bill.front_seconds>0；行：应赚工资/摸鱼成本按 moneyConfigured 隐藏、加班费恒显、出勤 work_days==null→"—"；7 日金条 max 归一 */ }
function paintDash(bill) { /* KPI+delta up/down 类+2×2 mini+最累最摸+双柱 bar-earn/bar-slack 3% 下限+dashFoot */ }
```

paintReceipt / paintDash 细节按设计文档渲染规则实现（金句四档、7 日金条、双柱等）。

- `load()`：L83 retention 后、`lastSaved = JSON.stringify(readCfg())` 前插 `setBillStyleUI(cfg.bill_style || "receipt");`
- `readCfg()`：cfg 对象 retention 后加 `bill_style: readBillStyle(),`
- `showView()`：`if (id === "viewMain") resetHistDates();` 后插 `if (id === "viewBill") loadWeekBill();`
- 绑定：

```js
$("billPrevWeek").addEventListener("click", () => shiftWeek(1));
$("billNextWeek").addEventListener("click", () => shiftWeek(-1));
const billStyleSeg = $("billStyleSeg");
if (billStyleSeg) billStyleSeg.querySelectorAll(".mon-seg-item").forEach(b =>
  b.addEventListener("click", () => {
    setBillStyleUI(b.dataset.bill);
    saveIfChanged();
    paintWeekBill();   // 已有 billData 缓存，切风格免重拉
  }));
```

- 轮询区不接账单（周级聚合）
- FE_VER 换新戳（与 index.html 两处一致）

### D3. styles.css（文件尾追加，字面色——styles.css 无 CSS 变量）

小票：白底 `#fdfdf8` 纸质、锯齿边（linear-gradient 三角）、等宽金额、虚线分隔、红色圆章「已验讫」（rotate(-14deg)、border 2px #c0392b、opacity .85）。仪表盘：KPI 大金额金色 #ffd650、mini-grid 2×2、双柱 bar-earn 金 / bar-slack 灰红。空态：rail-ico 同字号大图标 + 灰文案。深色 body #1a1a1c 上小票用纸白形成对比。

## Task E：验收 + 提交推送

1. `node scripts/test_week_bill.js`（GREEN）
2. `node scripts/test_topnav.js`、`test_commands.js`、`test_slacking.js` 等全量回归（cmd 循环跑 scripts\test_*.js）
3. `cargo test`（weekbill 单测）
4. `.\build.bat debug`（编译 + capabilities 自动补 + 缓存戳同步；确认 allow-get-week-bill 生成）
5. CHANGELOG.md 加条目
6. `git add -A` → commit → **push**（goal 已授权）

---

## 口径速查（实现中随时对照）

| 口径 | 规则 |
|---|---|
| 未来周 | week_offset 封顶 0，下一周按钮 disabled |
| 今天 | calc::compute().earned 实时；未来 0；休息日 0 |
| 满勤 | 月薪 ÷ 当月工作日数（每天按自己所属月的分母） |
| 点击 | left − 2×dbl + dbl + right + mid + xbtn（滚轮不计）＝ left−dbl+right+mid+xbtn |
| act_events | moves+left+dbl+right+wheel+mid+xbtn+keys（= total_events 口径） |
| 摸鱼率 | 分子分母均只含工作日；休息日不计 |
| slack_cost | Σ(摸鱼秒/3600 × 当日时薪)；当日时薪 = 当日 salary / daily_hours |
| 环比 | delta_pct 百分点；prev≤0 → None |
| 跨年周 | is_workday/月工作日按天的年份取内置表；不触发网络 |
| work_days | 有打卡记录的工作日数；监控全关 → null（出勤显示 "—"） |
| 金句门槛 | front_seconds > 0 才显示（全零但 rate=0 时不误判） |
