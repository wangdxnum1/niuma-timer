const WEEK_BILL_QUIPS = [
  { min: 40, text: "本周工资建议原路退回" },
  { min: 25, text: "将近三分之一的班，上给了手机" },
  { min: 10, text: "摸得克制，装得敬业" },
  { min: 0, text: "本周天选牛马，老板的战略合作伙伴" },
];

function weekBillQuip(ratePct, withMoney, span) {
  const tiers = withMoney ? WEEK_BILL_QUIPS : WEEK_BILL_QUIPS.filter((q) => q.min < 40);
  for (const q of tiers) {
    if (ratePct >= q.min) return spanQuipText(q.text, span);
  }
  return spanQuipText(tiers[tiers.length - 1].text, span);
}

// 月/年跨度账单复用同一组金句，仅把「本周」措辞换成「本月/今年」
function spanQuipText(text, span) {
  if (span === "month") return text.replace(/本周/g, "本月");
  if (span === "year") return text.replace(/本周/g, "今年");
  return text;
}

let weekOffset = 0; // 0=本周；上一周方向递增，未来周封顶
let billData = null; // 最近一次 WeekBill 缓存，切风格时免重拉
let reportBill = null; // 月报最近一次 PeriodBill（month 跨度），导出图片用
let reportLeaveCount = 0; // 月报对应的休假天数（day_override 标记数）
let billStyle = "receipt";
let curBillSpan = "week";
let billRequestGeneration = 0;
let billDataPeriod = null;
let reportBillPeriod = null;

// Cached data may only be exported for the exact period that produced it.
function billPeriodKey() {
  return curBillSpan + ":" + weekOffset;
}

function billIsReady() {
  return !!billData && billDataPeriod === billPeriodKey();
}

function reportIsReady() {
  return !!reportBill && reportBillPeriod === billPeriodKey();
}

// Loading/error state replaces the old body, so labels and amounts cannot disagree.
function setBillLoadState(kind, state) {
  const report = kind === "report";
  const ready = state === "ready";
  const failed = state === "error";
  const hint = $(report ? "reportSpanHint" : "billEmpty");
  $(report ? "reportRetryBtn" : "billRetryBtn").classList.toggle("hidden", !failed);
  $(report ? "reportImageBtn" : "billImageBtn").disabled = !ready;
  if (!report) $("billExportBtn").disabled = !ready;
  if (!ready) {
    if (report) {
      reportBill = null;
      reportBillPeriod = null;
      reportLeaveCount = 0;
      $("reportBody").classList.add("hidden");
    } else {
      billData = null;
      billDataPeriod = null;
      $("billReceipt").classList.add("hidden");
      $("billDash").classList.add("hidden");
    }
    hint.classList.remove("hidden");
    hint.querySelector(".bill-empty-txt").textContent =
      (report ? "报告" : "账单") + (failed ? "加载失败，请重试" : "加载中…");
    $("billWeekLabel").textContent = failed ? "加载失败" : "加载中…";
    $("billNextWeek").disabled = true;
  } else {
    hint.querySelector(".bill-empty-txt").textContent = "这一期还没有打工记录";
    if (report) hint.classList.add("hidden");
  }
}

// 有效时薪未配置：工资与摸鱼成本口径不成立，相关行一律隐藏。
// 统一口径「有效时薪 > 0」：月聘看月薪输入框，时薪模式（v1.6.0）看时薪输入框
function moneyConfigured() {
  const seg = document.querySelector("#salaryModeSeg .mon-seg-item.active");
  if (seg && seg.dataset.salaryMode === "hourly") {
    return parseFloat($("hourly_wage").value) > 0;
  }
  return parseFloat($("monthly_salary").value) > 0;
}

function readBillStyle() {
  const active = document.querySelector("#billStyleSeg .bill-style-card.active");
  return active ? active.dataset.bill : "receipt";
}

function setBillStyleUI(style) {
  billStyle = style === "dashboard" ? "dashboard" : "receipt";
  document.querySelectorAll("#billStyleSeg .bill-style-card").forEach((b) => {
    const on = b.dataset.bill === billStyle;
    b.classList.toggle("active", on);
    b.setAttribute("aria-checked", on ? "true" : "false");
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
  setBillLoadState("bill", "loading");
  const generation = ++billRequestGeneration;
  const span = curBillSpan, offset = weekOffset, tab = curBillTab;
  const isCurrent = () => generation === billRequestGeneration && span === curBillSpan && offset === weekOffset && tab === curBillTab && curView === "viewBill";
  try {
    const result = await invoke("get_bill", { span: curBillSpan, offset: weekOffset });
    if (!isCurrent()) return;
    billData = result;
    billDataPeriod = billPeriodKey();
    setBillLoadState("bill", "ready");
    paintWeekBill();
  } catch (e) {
    if (!isCurrent()) return;
    flog("get_bill ERR: " + (e && e.message ? e.message : String(e)));
    setBillLoadState("bill", "error");
  }
}

function shiftWeek(delta) {
  if (curBillTab === "timeline") return;
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
    quote.textContent = weekBillQuip((slackSecs / front) * 100, moneyConfigured(), curBillSpan);
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


// ---- 周报图片（v1.6.0）：把当前周期账单画成一张 PNG ----

// 摸鱼换算（设置 → 外观的单位/单价 → 人话）。报告卡与导图共用。
function currentSlackEquiv(cost) {
  return fmtSlackEquiv(
    cost,
    ($("slack_equiv_unit") || {}).value || "milktea",
    ($("slack_equiv_price") || {}).value
  );
}

// PeriodBill → 图片模型（纯函数，测试直接断言折算口径）。leaveDays：报告的
// 休假天数（day_override 标记数），月/年报告传入；slackEquiv：摸鱼换算文案。
function buildReportModel(bill, leaveDays, slackEquiv) {
  const hasFront = (bill.front_seconds || 0) > 0;
  const ratePct = hasFront ? (bill.slack_rate || 0) * 100 : null;
  return {
    label: bill.period_label || "",
    range: fmtDateRange(bill.period_start, bill.period_end),
    income: bill.total_income || 0,
    base: bill.base_salary || 0,
    ot: bill.ot_fee || 0,
    otHours: bill.ot_hours || 0,
    slackCost: hasFront && moneyConfigured() ? bill.slack_cost || 0 : null,
    slackPct: ratePct,
    workDays: bill.work_days,
    workHours: bill.work_hours || 0,
    leaveDays: leaveDays == null ? null : leaveDays,
    slackEquiv: hasFront && moneyConfigured() ? slackEquiv || "" : "",
    bars: (bill.buckets || []).map((b) => ({
      label: b.label,
      salary: b.salary || 0,
      isWorkday: b.is_workday !== false,
    })),
    hardest: bill.hardest || null,
    slackiest: bill.slackiest || null,
    span: curBillSpan,
    quip: hasFront ? weekBillQuip(ratePct, moneyConfigured(), curBillSpan) : "",
  };
}

// canvas 手绘报告卡：750×1050 逻辑尺寸、2x 物理密度保证文字锐利
function drawReport(model, scale) {
  const s = scale || 2;
  const canvas = document.createElement("canvas");
  canvas.width = 750 * s;
  canvas.height = 1050 * s;
  const ctx = canvas.getContext("2d");
  ctx.scale(s, s);
  const GOLD = "#ffd650";
  const GRAY = "rgba(242,242,244,0.55)";
  const HAIR = "rgba(255,255,255,0.09)";
  ctx.fillStyle = "#1a1a1c";
  ctx.fillRect(0, 0, 750, 1050);
  // 顶部品牌金条
  ctx.fillStyle = GOLD;
  ctx.fillRect(0, 0, 750, 6);

  ctx.textBaseline = "alphabetic";
  ctx.fillStyle = GOLD;
  ctx.font = '600 26px "Segoe UI", "Microsoft YaHei", sans-serif';
  const spanTitle = { month: "月账单", year: "年账单" }[model.span] || "周账单";
  ctx.fillText("牛马计时器 · " + spanTitle, 55, 92);
  ctx.fillStyle = "#f2f2f4";
  ctx.font = '400 21px "Segoe UI", "Microsoft YaHei", sans-serif';
  ctx.fillText(model.label + " · " + model.range, 55, 130);

  ctx.fillStyle = GRAY;
  ctx.font = '400 18px "Segoe UI", "Microsoft YaHei", sans-serif';
  ctx.fillText("本期进账", 55, 185);
  ctx.fillStyle = "#ffffff";
  ctx.font = '700 62px "Segoe UI", "Microsoft YaHei", sans-serif';
  ctx.fillText(fmtMoney(model.income), 55, 248);

  // 明细（基础 / 加班 / 摸鱼 / 摸鱼率 + 换算）；报告补一行休假天数（月/年）
  const rows = [
    ["基础工资", fmtMoney(model.base)],
    ["加班费", model.ot > 0 ? fmtMoney(model.ot) + "（" + model.otHours.toFixed(1) + "h）" : "—"],
    ["摸鱼成本", model.slackCost == null ? "—" : fmtMoney(model.slackCost)],
    ["摸鱼率", model.slackPct == null ? "暂无记录" : model.slackPct.toFixed(1) + "%" + (model.slackEquiv ? " " + model.slackEquiv : "")],
  ];
  if (model.span !== "week" && model.leaveDays != null) {
    rows.splice(3, 0, ["休假", model.leaveDays + " 天"]);
  }
  let y = 305;
  rows.forEach(([k, v]) => {
    ctx.fillStyle = HAIR;
    ctx.fillRect(55, y - 26, 640, 1);
    ctx.fillStyle = GRAY;
    ctx.font = '400 19px "Segoe UI", "Microsoft YaHei", sans-serif';
    ctx.fillText(k, 55, y);
    ctx.fillStyle = "#f2f2f4";
    ctx.font = '600 19px "Segoe UI", "Microsoft YaHei", sans-serif';
    ctx.fillText(v, 695 - ctx.measureText(v).width, y);
    y += 46;
  });
  ctx.fillStyle = GRAY;
  ctx.font = '400 15px "Segoe UI", "Microsoft YaHei", sans-serif';
  const wd = model.workDays == null ? "" : "工作日 " + model.workDays + " 天 · ";
  ctx.fillText(wd + "工时 " + model.workHours.toFixed(1) + "h", 55, y - 12);

  // 每日进账金条：salary 归一，休息日细半透明柱
  ctx.fillStyle = GRAY;
  ctx.font = '400 18px "Segoe UI", "Microsoft YaHei", sans-serif';
  ctx.fillText(model.span === "year" ? "每月进账" : "每日进账", 55, 545);
  const bars = model.bars || [];
  const n = Math.max(1, bars.length);
  const areaL = 55,
    areaR = 695,
    baseY = 760,
    maxH = 170;
  const gap = Math.min(14, Math.max(2, (areaR - areaL) / n * 0.25));
  const slotW = (areaR - areaL) / n;
  const barW = Math.min(52, slotW - gap);
  const maxSal = Math.max(0.01, ...bars.map((b) => b.salary));
  bars.forEach((b, i) => {
    const x = areaL + i * slotW + (slotW - barW) / 2;
    const h = (b.salary / maxSal) * maxH;
    if (h > 0) {
      if (b.isWorkday) {
        const g = ctx.createLinearGradient(0, baseY - h, 0, baseY);
        g.addColorStop(0, "#ffd650");
        g.addColorStop(1, "#ff9f0a");
        ctx.fillStyle = g;
        ctx.fillRect(x, baseY - h, barW, h);
      } else {
        ctx.fillStyle = "rgba(255,255,255,0.22)";
        ctx.fillRect(x, baseY - h, barW, h);
      }
    } else {
      ctx.fillStyle = "rgba(255,255,255,0.08)";
      ctx.fillRect(x, baseY - 3, barW, 3);
    }
    // 桶标签：≤12 桶直接标（周 7 / 年 12），月视图 30 桶密度太高不标
    if (n <= 12) {
      ctx.fillStyle = "rgba(242,242,244,0.45)";
      ctx.font = '400 12px "Segoe UI", "Microsoft YaHei", sans-serif';
      const t = b.label || "";
      ctx.fillText(t, x + barW / 2 - ctx.measureText(t).width / 2, baseY + 20);
    }
  });
  ctx.fillStyle = HAIR;
  ctx.fillRect(areaL, baseY, areaR - areaL, 1);

  // 极值两行 + 金句 + 页脚
  ctx.font = '400 17px "Segoe UI", "Microsoft YaHei", sans-serif';
  ctx.fillStyle = GRAY;
  ctx.fillText(
    model.hardest ? "最累 " + model.hardest.weekday + " · 加班 " + model.hardest.ot_hours.toFixed(1) + "h" : "",
    55,
    830
  );
  ctx.fillText(
    model.slackiest ? "最摸 " + model.slackiest.weekday + " · 摸鱼率 " + (model.slackiest.rate * 100).toFixed(1) + "%" : "",
    55,
    862
  );
  ctx.fillStyle = GOLD;
  ctx.font = 'italic 600 24px "Segoe UI", "Microsoft YaHei", sans-serif';
  ctx.fillText(model.quip, 55, 930);
  ctx.fillStyle = "rgba(242,242,244,0.35)";
  ctx.font = '400 14px "Segoe UI", "Microsoft YaHei", sans-serif';
  ctx.fillText("由牛马计时器生成", 55, 1010);
  return canvas;
}

// 保存链路：toBlob → 剪贴板（可用则复制）→ base64 → 后端落盘下载目录。
// 并发守卫同 downloadCsv：连点会重复画布 + 落盘一堆同名文件；按钮同步禁用给反馈。
let exportingImg = false;
// 画布 → PNG：剪贴板可用则先复制，再 base64 交后端落盘下载目录。
// 剪贴板不可用属预期（旧 WebView/权限）：静默降级为仅保存。账单图与月报图共用。
async function saveCanvasPng(canvas, filename) {
  const blob = await new Promise((res) => canvas.toBlob(res, "image/png"));
  if (!blob) {
    showToast("图片生成失败", "err");
    return;
  }
  let copied = false;
  try {
    if (typeof ClipboardItem === "function") {
      await navigator.clipboard.write([new ClipboardItem({ "image/png": blob })]);
      copied = true;
    }
  } catch (e) {
    /* fallthrough */
  }
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let bin = "";
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
  try {
    const path = await invoke("export_image", {
      filename,
      contentBase64: btoa(bin),
    });
    showToast(copied ? "已复制剪贴板，并保存到 " + path : "已保存到 " + path, "ok");
  } catch (e) {
    showToast("保存失败：" + e, "err");
  }
}

async function saveBillImage() {
  if (!billIsReady()) {
    showToast("先看一眼本期账单，加载完成后再来生成图片", "err");
    return;
  }
  if (exportingImg) return;
  exportingImg = true;
  const btn = $("billImageBtn");
  if (btn) btn.disabled = true;
  try {
    const model = buildReportModel(billData, null, currentSlackEquiv(billData.slack_cost || 0));
    const kind = { month: "月报", year: "年报" }[model.span] || "账单";
    await saveCanvasPng(drawReport(model), "niuma-" + kind + "-" + (model.label || billData.period_start) + ".png");
  } catch (e) {
    // buildReportModel/drawReport 抛错此前是无人处理的 rejection：按钮复位但
    // 无 toast 无日志（主窗口 unhandledrejection 兜底只管日志，可见反馈在这里补）
    flog("saveBillImage ERR: " + (e && e.message ? e.message : String(e)));
    showToast("账单图导出失败：" + (e && e.message ? e.message : e), "err");
  } finally {
    exportingImg = false;
    if (btn) btn.disabled = !billIsReady();
  }
}

// ---- 报告 tab：月报/年报战绩卡（PeriodBill + day_override 休假天数）----
// 数据与「本期账单」同源（get_bill 的 month/year 跨度）；周跨度给切换引导不拉数据
async function loadMonthlyReport() {
  if (curView !== "viewBill") return;
  loadMilestones(); // 里程碑与跨度无关，进报告页即拉（失败静默留旧值）
  setBillLoadState("report", "loading");
  const hint = $("reportSpanHint");
  const body = $("reportBody");
  if (curBillSpan !== "month" && curBillSpan !== "year") {
    hint.querySelector(".bill-empty-txt").textContent = "报告按月/年统计——先在上方把跨度切到「月」或「年」";
    hint.classList.remove("hidden");
    body.classList.add("hidden");
    // 期号标签跟当前跨度走（否则停留在上次报告的「N 月」标签，与跨度对不上）
    loadWeekBill();
    return;
  }
  const generation = ++billRequestGeneration;
  const span = curBillSpan, offset = weekOffset;
  const isCurrent = () =>
    generation === billRequestGeneration &&
    span === curBillSpan && offset === weekOffset &&
    curBillTab === "report" &&
    curView === "viewBill";
  try {
    const bill = await invoke("get_bill", { span, offset });
    if (!isCurrent()) return;
    const leaves = await invoke("get_day_overrides", {
      start: String(bill.period_start || ""),
      end: String(bill.period_end || ""),
    });
    if (!isCurrent()) return;
    // Publish navigation, body and export data together, after both reads succeed.
    $("billWeekLabel").textContent = bill.period_label + " · " + fmtDateRange(bill.period_start, bill.period_end);
    $("billNextWeek").disabled = !!bill.is_current_period;
    reportBillPeriod = billPeriodKey();
    setBillLoadState("report", "ready");
    paintReport(bill, leaves);
  } catch (e) {
    if (!isCurrent()) return;
    flog("loadReport ERR: " + (e && e.message ? e.message : String(e)));
    setBillLoadState("report", "error");
  }
}

function paintReport(bill, leaves) {
  const hasRecord = (bill.front_seconds || 0) > 0 || (bill.ot_hours || 0) > 0 ||
    (bill.ot_fee || 0) > 0 || leaves.length > 0 ||
    (bill.buckets || []).some((b) => b.has_record);
  const hint = $("reportSpanHint");
  if (!hasRecord) {
    reportBill = null;
    reportBillPeriod = null;
    reportLeaveCount = 0;
    hint.querySelector(".bill-empty-txt").textContent = "这一期还没有记录";
    hint.classList.remove("hidden");
    $("reportBody").classList.add("hidden");
    $("reportImageBtn").disabled = true;
    return;
  }
  hint.classList.add("hidden");
  reportBill = bill;
  reportLeaveCount = leaves.length;
  $("reportTitle").textContent = (bill.period_label || "本期") + "战绩";
  $("reportRange").textContent = fmtDateRange(bill.period_start, bill.period_end);
  $("reportIncome").textContent = fmtMoney(bill.total_income);
  const md = (iso) => String(iso || "").slice(5).replace("-", ".");
  const rows = [];
  rows.push([
    "出勤",
    bill.work_days == null ? "—" : bill.work_days + " 天 · 在岗约 " + (bill.work_hours || 0).toFixed(1) + "h",
  ]);
  rows.push(["休假", leaves.length ? leaves.map((x) => md(x.date)).join("、") + "（" + leaves.length + " 天）" : "无"]);
  rows.push(["加班", (bill.ot_hours || 0) > 0 ? (bill.ot_hours || 0).toFixed(1) + "h · " + fmtMoney(bill.ot_fee) : "—"]);
  const front = bill.front_seconds || 0;
  const equiv = moneyConfigured() ? currentSlackEquiv(bill.slack_cost || 0) : "";
  rows.push(["摸鱼率", front > 0 ? ((bill.slack_rate || 0) * 100).toFixed(1) + "%" + (equiv ? " " + equiv : "") : "暂无记录"]);
  if (bill.hardest) {
    rows.push(["最拼一天", md(bill.hardest.date) + " " + bill.hardest.weekday + " · 加班 " + (bill.hardest.ot_hours || 0).toFixed(1) + "h"]);
  }
  if (bill.slackiest) {
    rows.push(["最摸一天", md(bill.slackiest.date) + " " + bill.slackiest.weekday + " · " + ((bill.slackiest.rate || 0) * 100).toFixed(1) + "%"]);
  }
  // 年报特有：最拼月（后端给 12 个月桶；天粒度 hardest 之外按月再取一档）
  if (curBillSpan === "year") {
    const monthOt = (bill.buckets || []).filter((b) => (b.ot_total || 0) > 0);
    if (monthOt.length) {
      const top = monthOt.reduce((a, b) => ((b.ot_total || 0) > (a.ot_total || 0) ? b : a));
      rows.push(["最拼月", top.label + " · 加班费 " + fmtMoney(top.ot_total)]);
    }
  }
  const box = $("reportRows");
  box.textContent = "";
  rows.forEach(([k, v]) => {
    const row = document.createElement("div");
    row.className = "report-row";
    const l = document.createElement("span");
    l.textContent = k;
    const vEl = document.createElement("span");
    vEl.textContent = v;
    row.append(l, vEl);
    box.appendChild(row);
  });
  const q = $("reportQuote");
  if (front > 0) {
    q.textContent = weekBillQuip((bill.slack_rate || 0) * 100, moneyConfigured(), curBillSpan);
    q.classList.remove("hidden");
  } else {
    q.classList.add("hidden");
  }
  $("reportBody").classList.remove("hidden");
}

let exportingReport = false;
async function saveReportImage() {
  if (curBillSpan !== "month" && curBillSpan !== "year") {
    showToast("报告按月/年统计——先切到「月」或「年」跨度", "err");
    return;
  }
  if (!reportIsReady()) {
    showToast("报告还没就绪，稍等一下再试", "err");
    return;
  }
  if (exportingReport) return;
  exportingReport = true;
  const btn = $("reportImageBtn");
  if (btn) btn.disabled = true;
  try {
    const model = buildReportModel(
      reportBill,
      reportLeaveCount,
      currentSlackEquiv(reportBill.slack_cost || 0)
    );
    const kind = { month: "月报", year: "年报" }[model.span] || "报告";
    await saveCanvasPng(drawReport(model), "niuma-" + kind + "-" + (model.label || reportBill.period_start) + ".png");
  } catch (e) {
    flog("saveReportImage ERR: " + (e && e.message ? e.message : String(e)));
    showToast("报告图导出失败：" + (e && e.message ? e.message : e), "err");
  } finally {
    exportingReport = false;
    if (btn) btn.disabled = !reportIsReady();
  }
}

// ---- 里程碑：用这台应用以来的累计成就（后端出原始量，阶梯在前端）----
// px/km 与 core.js fmtDist 同一口径（96dpi）；里程碑阈值调档只改这里
const PX_PER_KM = (96 * 100 * 1000) / 2.54;
const MILESTONE_LADDERS = [
  { name: "加班时长", get: (m) => m.ot_hours || 0, steps: [100, 500, 1000], fmt: (v) => v.toFixed(0) + " h", stepFmt: (s) => s + " h" },
  { name: "键盘敲击", get: (m) => m.keystrokes || 0, steps: [1e6, 5e6, 1e7], fmt: (v) => fmtWan(v) + " 次", stepFmt: (s) => fmtWan(s) + " 次" },
  { name: "键鼠移动", get: (m) => m.distance_px || 0, steps: [10 * PX_PER_KM, 50 * PX_PER_KM, 100 * PX_PER_KM], fmt: (v) => fmtDist(v), stepFmt: (s) => fmtDist(s) },
  { name: "专注时长", get: (m) => m.focus_minutes || 0, steps: [3000, 12000, 30000], fmt: (v) => fmtDurCN(v * 60), stepFmt: (s) => fmtDurCN(s * 60) },
  { name: "相伴天数", get: (m) => m.active_days || 0, steps: [30, 100, 365], fmt: (v) => v + " 天", stepFmt: (s) => s + " 天" },
];

let milestonesData = null;
async function loadMilestones() {
  try {
    milestonesData = await invoke("get_milestones");
    paintMilestones();
  } catch (e) {
    // 失败静默留旧值：里程碑是锦上添花，不值得为它打断报告页
    flog("get_milestones ERR: " + (e && e.message ? e.message : String(e)));
  }
}

function paintMilestones() {
  const box = $("milestonesBox");
  const list = $("milestonesList");
  if (!box || !list || !milestonesData) return;
  box.classList.remove("hidden");
  list.textContent = "";
  for (const ladder of MILESTONE_LADDERS) {
    const value = ladder.get(milestonesData);
    const achieved = ladder.steps.filter((s) => value >= s).length;
    const next = ladder.steps[achieved] || null;
    const pct = next ? Math.min(100, (value / next) * 100) : 100;
    const row = document.createElement("div");
    row.className = "ms-row";
    const name = document.createElement("span");
    name.className = "ms-name";
    name.textContent = ladder.name;
    const bar = document.createElement("div");
    bar.className = "ms-bar";
    const fill = document.createElement("i");
    fill.style.width = pct.toFixed(1) + "%";
    bar.appendChild(fill);
    const val = document.createElement("span");
    val.className = "ms-val";
    val.textContent = ladder.fmt(value);
    const done = document.createElement("span");
    done.className = "ms-done" + (achieved >= ladder.steps.length ? " full" : "");
    done.textContent =
      (next ? "下一档 " + ladder.stepFmt(next) : "已满档") +
      " · " + achieved + "/" + ladder.steps.length;
    row.append(name, bar, val, done);
    list.appendChild(row);
  }
}
