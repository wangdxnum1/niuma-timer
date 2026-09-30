const WEEK_BILL_QUIPS = [
  { min: 40, text: "本周工资建议原路退回" },
  { min: 25, text: "将近三分之一的班，上给了手机" },
  { min: 10, text: "摸得克制，装得敬业" },
  { min: 0, text: "本周天选牛马，老板的战略合作伙伴" },
];

function weekBillQuip(ratePct, withMoney) {
  const tiers = withMoney ? WEEK_BILL_QUIPS : WEEK_BILL_QUIPS.filter((q) => q.min < 40);
  for (const q of tiers) {
    if (ratePct >= q.min) return q.text;
  }
  return tiers[tiers.length - 1].text;
}

let weekOffset = 0; // 0=本周；上一周方向递增，未来周封顶
let billData = null; // 最近一次 WeekBill 缓存，切风格时免重拉
let billStyle = "receipt";
let curBillSpan = "week";
let billRequestGeneration = 0;

function fmtMoney(n) {
  return "¥" + (Number(n) || 0).toFixed(2);
}

// 月薪未配（输入框空/0）：工资与摸鱼成本口径不成立，相关行一律隐藏
function moneyConfigured() {
  return parseFloat($("monthly_salary").value) > 0;
}

function readBillStyle() {
  const active = document.querySelector("#billStyleSeg .mon-seg-item.active");
  return active ? active.dataset.bill : "receipt";
}

function setBillStyleUI(style) {
  billStyle = style === "dashboard" ? "dashboard" : "receipt";
  document.querySelectorAll("#billStyleSeg .mon-seg-item").forEach((b) => {
    b.classList.toggle("active", b.dataset.bill === billStyle);
  });
}

function readBillSpan() {
  const active = document.querySelector("#billSpanSeg .mon-seg-item.active");
  return active ? active.dataset.span : "week";
}

function setBillSpanUI(span) {
  curBillSpan = span === "month" || span === "year" ? span : "week";
  document.querySelectorAll("#billSpanSeg .mon-seg-item").forEach((b) => {
    b.classList.toggle("active", b.dataset.span === curBillSpan);
  });
}

async function loadWeekBill() {
  if (curView !== "viewBill") return;
  const generation = ++billRequestGeneration;
  const span = curBillSpan, offset = weekOffset, tab = curBillTab;
  const isCurrent = () => generation === billRequestGeneration && span === curBillSpan && offset === weekOffset && tab === curBillTab && curView === "viewBill";
  try {
    const result = await invoke("get_bill", { span: curBillSpan, offset: weekOffset });
    if (!isCurrent()) return;
    billData = result;
    paintWeekBill();
  } catch (e) {
    if (!isCurrent()) return;
    flog("get_bill ERR: " + (e && e.message ? e.message : String(e)));
    $("billWeekLabel").textContent = "账单加载失败";
  }
}

function shiftWeek(delta) {
  const next = weekOffset + delta;
  if (next < 0) return; // 本周封顶，不预看未来
  weekOffset = next;
  loadBillTab(); // 翻周联动当前 tab（账单/热力/趋势/身体），不总是重拉账单
}

function paintWeekBill() {
  const bill = billData;
  if (!bill || curView !== "viewBill") return;
  const start = String(bill.period_start || "");
  const end = String(bill.period_end || "");
  $("billWeekLabel").textContent = bill.period_label + " · " + fmtDateRange(start, end);
  $("billNextWeek").disabled = !!bill.is_current_period;
  // 整周零记录 → 空态：工资虽是推算的，但没有任何监控证据就不评判，不排一排 ¥0.00
  const anyRecord = (bill.buckets || []).some((d) => d.has_record);
  $("billEmpty").classList.toggle("hidden", anyRecord);
  $("billReceipt").classList.toggle("hidden", !anyRecord || billStyle !== "receipt");
  $("billDash").classList.toggle("hidden", !anyRecord || billStyle !== "dashboard");
  if (!anyRecord) return;
  if (billStyle === "receipt") paintReceipt(bill);
  else paintDash(bill);
}

function billLine(label, value) {
  const div = document.createElement("div");
  div.className = "rcp-line";
  const l = document.createElement("span");
  l.textContent = label;
  const v = document.createElement("span");
  v.textContent = value;
  div.append(l, v);
  return div;
}

// 休息日/未来日：金条位置放占位字
function offDay(text) {
  const t = document.createElement("div");
  t.className = "rcp-day-off";
  t.textContent = text;
  return t;
}

function paintReceipt(bill) {
  $("rcpNo").textContent = "NO." + String(bill.period_start || "").replace(/-/g, "");
  $("rcpAmount").textContent = fmtMoney(bill.total_income);

  // 金句门槛：front_seconds>0（无任何应用记录不评判）；摸鱼率=摸鱼秒÷四类前台总秒
  const front = bill.front_seconds || 0;
  const slackSecs = (bill.buckets || []).reduce((s, d) => s + (d.slack_seconds || 0), 0);
  const quote = $("rcpQuote");
  if (front > 0) {
    quote.textContent = weekBillQuip((slackSecs / front) * 100, moneyConfigured());
    quote.classList.remove("hidden");
  } else {
    quote.classList.add("hidden");
  }

  // 小票行：应赚工资 / 加班费 / 摸鱼成本 / 出勤工时
  const lines = $("rcpLines");
  lines.textContent = "";
  if (moneyConfigured()) lines.append(billLine("应赚工资", fmtMoney(bill.base_salary)));
  lines.append(billLine("加班费", fmtMoney(bill.ot_fee)));
  if (moneyConfigured() && $("monitor_app_usage").checked)
    lines.append(billLine("摸鱼成本", fmtMoney(bill.slack_cost)));
  lines.append(
    billLine(
      "出勤工时",
      bill.work_days == null ? "—" : "在岗约 " + (bill.work_hours || 0).toFixed(1) + "h"
    )
  );

  // 7 日金条：满勤日画金条（按周内最大工资归一，保底 8%），休息/未来/未到特殊态
  const box = $("rcpDays");
  box.textContent = "";
  const today = todayStr();
  const maxSalary = Math.max(0.01, ...(bill.buckets || []).map((d) => d.salary || 0));
  (bill.buckets || []).forEach((d) => {
    const cell = document.createElement("div");
    cell.className = "rcp-day";
    if (d.is_workday === false) {
      cell.append(offDay("休"));
    } else if (d.date && String(d.date) > today) {
      cell.append(offDay("未到"));
    } else {
      const wrap = document.createElement("div");
      wrap.className = "rcp-day-bar";
      const fill = document.createElement("div");
      fill.className = "rcp-day-fill";
      fill.style.height = Math.max(8, ((d.salary || 0) / maxSalary) * 100) + "%";
      wrap.append(fill);
      cell.append(wrap);
    }
    const wd = document.createElement("div");
    wd.className = "rcp-day-wd";
    wd.textContent = d.label || "";
    const num = document.createElement("div");
    num.className = "rcp-day-date";
    num.textContent = String(d.date || "").slice(8);
    cell.append(wd, num);
    box.append(cell);
  });

  $("rcpFootNote").textContent = $("monitor_activity").checked
    ? "键鼠 " + (bill.keys_total || 0) + " 次 · 点击 " + (bill.clicks_total || 0) + " 次"
    : "键鼠监控未开启";
}

function paintDash(bill) {
  $("kpiAmount").textContent = fmtMoney(bill.total_income);
  const delta = $("kpiDelta");
  if (bill.delta_pct == null) {
    delta.textContent = ""; // 无上周基数不显示箭头，不出现 ∞
    delta.className = "kpi-delta";
  } else {
    const up = bill.delta_pct >= 0;
    delta.textContent = (up ? "▲ " : "▼ ") + Math.abs(bill.delta_pct).toFixed(1) + "% vs 上期";
    delta.className = "kpi-delta " + (up ? "up" : "down");
  }

  $("miniSalary").textContent = moneyConfigured() ? fmtMoney(bill.base_salary) : "—";
  $("miniOt").textContent = fmtMoney(bill.ot_fee);
  $("miniSlack").textContent =
    moneyConfigured() && $("monitor_app_usage").checked ? fmtMoney(bill.slack_cost) : "—";
  $("miniHours").textContent =
    bill.work_days == null ? "—" : (bill.work_hours || 0).toFixed(1) + "h";

  const ext = [];
  if (bill.hardest)
    ext.push("最累 " + bill.hardest.weekday + " 加班 " + (bill.hardest.ot_hours || 0).toFixed(1) + "h");
  if ($("monitor_app_usage").checked && bill.slackiest)
    ext.push(
      "最摸 " + bill.slackiest.weekday + " 摸鱼率 " + Math.round((bill.slackiest.rate || 0) * 100) + "%"
    );
  $("dashExtreme").textContent = ext.join(" · ");

  // 每日双柱：入账(金)/摸鱼(红)各按自身最大值归一（保底 3%），休息/未来留空
  const bars = $("dashBars");
  bars.textContent = "";
  const today = todayStr();
  const maxEarn = Math.max(0.01, ...(bill.buckets || []).map((d) => (d.salary || 0) + (d.ot_total || 0)));
  const maxSlack = Math.max(0.01, ...(bill.buckets || []).map((d) => d.slack_seconds || 0));
  (bill.buckets || []).forEach((d) => {
    const col = document.createElement("div");
    col.className = "bar-col";
    const pair = document.createElement("div");
    pair.className = "bar-pair";
    if (d.is_workday !== false && (!d.date || String(d.date) <= today)) {
      const earn = document.createElement("div");
      earn.className = "bar bar-earn";
      earn.style.height = Math.max(3, (((d.salary || 0) + (d.ot_total || 0)) / maxEarn) * 100) + "%";
      const slack = document.createElement("div");
      slack.className = "bar bar-slack";
      slack.style.height = Math.max(3, ((d.slack_seconds || 0) / maxSlack) * 100) + "%";
      pair.append(earn, slack);
    }
    const wd = document.createElement("div");
    wd.className = "bar-wd";
    wd.textContent = d.label || "";
    col.append(pair, wd);
    bars.append(col);
  });

  $("dashFoot").textContent = $("monitor_activity").checked
    ? "键鼠 " + (bill.keys_total || 0) + " 次 · 点击 " + (bill.clicks_total || 0) + " 次"
    : "键鼠监控未开启";
}

// ---- 洞察三视图（v1.2.0）：时段热力 / 周趋势 / 身体账单 ----
// 口径与周账单严格一致（见 src-tauri/src/insights.rs）：
// events = 键鼠全量（滚轮格数不计入）、clicks = 双击折算 1 次、
// slack_rate = 摸鱼秒 ÷ 前台秒（前台 0 → null，前端断线不画 0）

