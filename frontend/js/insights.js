const WD_CN = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"]; // Date.getDay() 索引

let curBillTab = "bill";
const BILL_TAB_PANES = {
  bill: "billTabBill",
  heat: "billTabHeat",
  trend: "billTabTrend",
  body: "billTabBody",
  timeline: "billTabTimeline",
};
const BILL_TAB_KEYS = ["bill", "heat", "trend", "body", "timeline"]; // 翻页器循环顺序
const BILL_TAB_NAMES = {
  bill: "本周账单",
  heat: "时段热力",
  trend: "趋势",
  body: "身体账单",
  timeline: "时间线",
};
let heatData = null; // 各 tab 最近一次返回缓存（仅当前 tab 会被重画）
let trendData = null;
let bodyData = null;

function setBillTabUI(tab) {
  curBillTab = BILL_TAB_PANES[tab] ? tab : "bill";
  // 账单翻页器（与明细同款）：页名跟随当前页，圆点只在 billPager 内点亮
  $("billPgName").textContent = BILL_TAB_NAMES[curBillTab];
  document.querySelectorAll("#billPager .pg-dot").forEach((b) => {
    b.classList.toggle("active", b.dataset.btab === curBillTab);
  });
  Object.entries(BILL_TAB_PANES).forEach(([key, id]) =>
    $(id).classList.toggle("hidden", key !== curBillTab)
  );
}

// 懒加载分发：进页 / 切 tab / 翻周才拉对应命令（周级聚合，不进 tick 轮询）
async function loadBillTab() {
  if (curBillTab === "bill") return loadWeekBill();
  if (curBillTab === "heat") return loadHourHeat();
  if (curBillTab === "trend") return loadWeekTrend();
  if (curBillTab === "timeline") return loadDayTimeline();
  return loadBodyBill();
}

// 日期范围标签：同年右端省年份；跨年双侧都带（12.29–01.04 不再看不出年份）。
function fmtDateRange(startIso, endIso) {
  const f = (iso, withYear) => {
    const md = String(iso || "").slice(5).replace("-", ".");
    return withYear ? String(iso || "").slice(0, 4) + "." + md : md;
  };
  const sy = String(startIso || "").slice(0, 4);
  const ey = String(endIso || "").slice(0, 4);
  return f(startIso, true) + "–" + f(endIso, sy !== ey);
}

// 账单页顶部周导航随当前 tab 的数据更新（后端已封顶未来周，周日晚 ≥ 今天即本周）
function paintBillNav(startIso, endIso) {
  $("billWeekLabel").textContent = fmtDateRange(startIso, endIso);
  $("billNextWeek").disabled = String(endIso) >= todayStr();
}

function fmtWan(n) {
  n = Number(n) || 0;
  return n >= 10000 ? (n / 10000).toFixed(1) + " 万" : String(n);
}

// ISO 日期（YYYY-MM-DD）+n 天，本地时区


// ---- 时段热力：7 行（周一~周日）× 24 列（0-23 时），sqrt 归一 4 级金色阶 ----

async function loadHourHeat() {
  if (curView !== "viewBill") return;
  const generation = ++billRequestGeneration;
  const span = curBillSpan, offset = weekOffset, tab = curBillTab;
  const isCurrent = () => generation === billRequestGeneration && span === curBillSpan && offset === weekOffset && tab === curBillTab && curView === "viewBill";
  try {
    const result = await invoke("get_heatmap", { span: curBillSpan, offset: weekOffset });
    if (!isCurrent()) return;
    heatData = result;
    paintHourHeat();
  } catch (e) {
    if (!isCurrent()) return;
    flog("get_heatmap ERR: " + (e && e.message ? e.message : String(e)));
    $("heatSummary").textContent = "热力图加载失败";
  }
}

// sqrt 归一拉开低值区分度：v≤0 → 0 级；>0 → 1+min(3, floor(sqrt(v/max)×4)) ∈ 1..4
function heatLevel(v, max) {
  if (v <= 0 || max <= 0) return 0;
  return 1 + Math.min(3, Math.floor(Math.sqrt(v / max) * 4));
}

function paintHourHeat() {
  const heat = heatData;
  if (!heat || curView !== "viewBill") return;
  paintBillNav(heat.week_start, heat.week_end);
  const cells = heat.cells || [];
  const hasCells = cells.length > 0;
  $("heatEmpty").classList.toggle("hidden", hasCells);
  $("heatSummary").classList.toggle("hidden", !hasCells);
  $("heatGrid").closest("section").classList.toggle("hidden", !hasCells);
  if (!hasCells) return;

  const total = cells.reduce((s, c) => s + (c.events || 0), 0);
  const summary = $("heatSummary");
  summary.textContent = "";
  const peak = document.createElement("b");
  peak.textContent = heat.peak_hour == null ? "—" : heat.peak_hour + " 时";
  summary.append("最忙时段 ", peak, " · 键鼠共 " + fmtWan(total) + " 次");

  // 列标 0-23 只建一次（静态内容，翻周不重建）
  const hours = $("heatHours");
  if (!hours.childElementCount) {
    for (let h = 0; h < 24; h++) {
      const t = document.createElement("span");
      t.className = "heat-hour-tick";
      t.textContent = h;
      hours.append(t);
    }
  }

  const maxEvents = Math.max(0, ...cells.map((c) => c.events || 0));
  const byKey = new Map(cells.map((c) => [c.weekday + " " + c.hour, c]));
  const grid = $("heatGrid");
  grid.textContent = "";
  for (let i = 0; i < 7; i++) {
    const rowWd = WD_CN[(i + 1) % 7];
    const rowLabel = document.createElement("span");
    rowLabel.className = "heat-axis";
    rowLabel.textContent = rowWd;
    grid.append(rowLabel);
    for (let h = 0; h < 24; h++) {
      const cell = document.createElement("div");
      const c = byKey.get(rowWd + " " + h);
      const ev = c ? c.events || 0 : 0;
      const lv = heatLevel(ev, maxEvents);
      cell.className = "heat-cell lv" + lv;
      if (c)
        cell.title =
          rowWd +
          " " +
          h +
          ":00 · 键鼠 " +
          fmtWan(ev) +
          " 次" +
          (c.front_secs > 0 ? " · 前台 " + (c.front_secs / 3600).toFixed(1) + "h" : "") +
          (c.audio_secs > 0 ? " · 在响 " + (c.audio_secs / 3600).toFixed(1) + "h" : "");
      grid.append(cell);
    }
  }
}

// ---- 周趋势：手写 SVG 折线（无外部库）——金线周入账，红细线摸鱼率，null 断线 ----

async function loadWeekTrend() {
  if (curView !== "viewBill") return;
  const generation = ++billRequestGeneration;
  const span = curBillSpan, offset = weekOffset, tab = curBillTab;
  const isCurrent = () => generation === billRequestGeneration && span === curBillSpan && offset === weekOffset && tab === curBillTab && curView === "viewBill";
  try {
    const result = await invoke("get_trend", { span: curBillSpan, offset: weekOffset });
    if (!isCurrent()) return;
    trendData = result;
    paintWeekTrend();
  } catch (e) {
    if (!isCurrent()) return;
    flog("get_trend ERR: " + (e && e.message ? e.message : String(e)));
    $("billWeekLabel").textContent = "趋势加载失败";
  }
}

const SVG_NS = "http://www.w3.org/2000/svg";

function svgEl(tag, attrs) {
  const el = document.createElementNS(SVG_NS, tag);
  for (const k in attrs) el.setAttribute(k, attrs[k]);
  return el;
}

function paintWeekTrend() {
  const trend = trendData;
  if (!trend || curView !== "viewBill") return;
  const pts = trend.points || [];
  const last = pts[pts.length - 1];
  if (last) paintBillNav(last.period_start, last.period_end);
  // 空窗口：八周全零收入且无前台记录（slack_rate 全 null）→ 不画一条趴地的 0 线
  const hasData = pts.some((p) => (p.income || 0) > 0 || p.slack_rate != null);
  $("trendSvg").closest("section").classList.toggle("hidden", !hasData);
  $("trendEmpty").classList.toggle("hidden", hasData);
  if (!hasData) return;

  // 绘图区：viewBox 640×220（preserveAspectRatio=none，px 换算 x/640·y/220 各自缩放）
  const L = 16,
    R = 624,
    T = 18,
    B = 196;
  const n = pts.length;
  const xAt = (i) => L + (i * (R - L)) / Math.max(1, n - 1);
  const maxIncome = Math.max(0, ...pts.map((p) => p.income || 0));
  const yIncome = (v) => B - (maxIncome > 0 ? (v / maxIncome) * (B - T) : 0);
  const ySlack = (r) => B - (r || 0) * (B - T);

  const svg = $("trendSvg");
  svg.textContent = "";
  // 底轴 + 三条半高参考线（弱化，只作视觉锚）
  svg.append(
    svgEl("line", { x1: L, y1: B, x2: R, y2: B, stroke: "rgba(255,255,255,0.12)", "stroke-width": 1 })
  );
  [0.25, 0.5, 0.75].forEach((g) =>
    svg.append(
      svgEl("line", {
        x1: L,
        y1: (B - g * (B - T)).toFixed(1),
        x2: R,
        y2: (B - g * (B - T)).toFixed(1),
        stroke: "rgba(255,255,255,0.05)",
        "stroke-width": 1,
      })
    )
  );

  // 金实线：周入账（口径同账单页 total_income；无记录周 0 照画）
  svg.append(
    svgEl("polyline", {
      points: pts.map((p, i) => xAt(i).toFixed(1) + "," + yIncome(p.income).toFixed(1)).join(" "),
      fill: "none",
      stroke: "#ffd650",
      "stroke-width": 2,
      "stroke-linejoin": "round",
      "stroke-linecap": "round",
    })
  );

  // 红细线：摸鱼率 0-1 归一；null（该周无前台记录）断线成多段
  const seg = [];
  const flushSeg = () => {
    if (seg.length > 1)
      svg.append(
        svgEl("polyline", {
          points: seg.join(" "),
          fill: "none",
          stroke: "#e0605f",
          "stroke-width": 1.5,
          opacity: 0.85,
        })
      );
    seg.length = 0;
  };
  pts.forEach((p, i) => {
    if (p.slack_rate == null) flushSeg();
    else seg.push(xAt(i).toFixed(1) + "," + ySlack(p.slack_rate).toFixed(1));
  });
  flushSeg();

  pts.forEach((p, i) => {
    svg.append(
      svgEl("circle", { cx: xAt(i).toFixed(1), cy: yIncome(p.income).toFixed(1), r: 3, fill: "#ffd650" })
    );
    if (p.slack_rate != null)
      svg.append(
        svgEl("circle", { cx: xAt(i).toFixed(1), cy: ySlack(p.slack_rate).toFixed(1), r: 2.5, fill: "#e0605f" })
      );
  });

  // x 轴周标签（M.D）
  const xa = $("trendXAxis");
  xa.textContent = "";
  pts.forEach((p) => {
    const s = document.createElement("span");
    s.textContent = p.label || "";
    xa.append(s);
  });

  // 悬停热区：每点一条竖带，最近邻取值 → trendTip（px 坐标按实际尺寸换算并防出边）
  const tip = $("trendTip");
  const zoneW = (R - L) / n;
  const wrap = $("trendSvg").parentElement;
  pts.forEach((p, i) => {
    const z = svgEl("rect", {
      x: (L + i * zoneW).toFixed(1),
      y: T,
      width: zoneW.toFixed(1),
      height: B - T,
      fill: "transparent",
      "pointer-events": "all",
    });
    z.addEventListener("mouseenter", () => {
      const range = fmtDateRange(p.period_start, p.period_end);
      const rate = p.slack_rate == null ? "—" : Math.round(p.slack_rate * 100) + "%";
      tip.textContent = range + " · 入账 " + fmtMoney(p.income) + " · 摸鱼率 " + rate;
      const px = Math.max(
        70,
        Math.min(wrap.clientWidth - 70, (xAt(i) / 640) * wrap.clientWidth)
      );
      const py = (yIncome(p.income) / 220) * wrap.clientHeight;
      tip.style.left = px.toFixed(0) + "px";
      tip.style.top = py.toFixed(0) + "px";
      tip.classList.remove("hidden");
    });
    z.addEventListener("mouseleave", () => tip.classList.add("hidden"));
    svg.append(z);
  });
}

// ---- 身体账单：一周键鼠损耗四指标 + 按天金柱（复用 dash-bars 单柱模式） ----

async function loadBodyBill() {
  if (curView !== "viewBill") return;
  const generation = ++billRequestGeneration;
  const span = curBillSpan, offset = weekOffset, tab = curBillTab;
  const isCurrent = () => generation === billRequestGeneration && span === curBillSpan && offset === weekOffset && tab === curBillTab && curView === "viewBill";
  try {
    const result = await invoke("get_body_bill", { span: curBillSpan, offset: weekOffset });
    if (!isCurrent()) return;
    bodyData = result;
    paintBodyBill();
  } catch (e) {
    if (!isCurrent()) return;
    flog("get_body_bill ERR: " + (e && e.message ? e.message : String(e)));
    $("billWeekLabel").textContent = "身体账单加载失败";
  }
}

// 像素 → 米（96dpi 估算：1in = 96px = 2.54cm）
function fmtMeters(px) {
  return (((Number(px) || 0) * 2.54) / 96 / 100).toFixed(1) + " m";
}

function paintBodyBill() {
  const body = bodyData;
  if (!body || curView !== "viewBill") return;
  paintBillNav(body.week_start, body.week_end);
  const days = body.days || [];
  const hasData = days.some((d) => (d.events || 0) > 0);
  $("bodyEmpty").classList.toggle("hidden", hasData);
  $("bodyClicks").closest(".body-grid").classList.toggle("hidden", !hasData);
  $("bodyBusiest").closest("section").classList.toggle("hidden", !hasData);
  if (!hasData) return;

  const recDays = Math.max(1, body.record_days || 0);
  $("bodyClicks").textContent = fmtWan(body.clicks) + " 次";
  $("bodyClicksAvg").textContent = "日均 " + fmtWan(Math.round(body.clicks / recDays)) + " 次";
  $("bodyKeys").textContent = fmtWan(body.keys) + " 次";
  $("bodyKeysAvg").textContent = "日均 " + fmtWan(Math.round(body.keys / recDays)) + " 次";
  $("bodyPixels").textContent = fmtMeters(body.pixels);
  $("bodyPixelsAvg").textContent = "日均 " + fmtMeters(body.pixels / recDays) + "（96dpi 估算）";
  $("bodyWheel").textContent = fmtWan(body.wheel_ticks) + " 格";
  $("bodyWheelAvg").textContent = "日均 " + fmtWan(Math.round(body.wheel_ticks / recDays)) + " 格";

  $("bodyBusiest").textContent = body.busiest
    ? "最累 " + body.busiest.weekday + " · 键鼠 " + fmtWan(body.busiest.events) + " 次"
    : "";

  // 按天键鼠单金柱：events 归一保底 3%，未来日不画
  const bars = $("bodyBars");
  bars.textContent = "";
  const today = todayStr();
  const maxEv = Math.max(0.01, ...days.map((d) => d.events || 0));
  days.forEach((d) => {
    const col = document.createElement("div");
    col.className = "bar-col";
    const pair = document.createElement("div");
    pair.className = "bar-pair";
    if (String(d.date) <= today && (d.events || 0) > 0) {
      const bar = document.createElement("div");
      bar.className = "bar bar-earn";
      bar.style.height = Math.max(3, ((d.events || 0) / maxEv) * 100) + "%";
      pair.append(bar);
    }
    const wd = document.createElement("div");
    wd.className = "bar-wd";
    wd.textContent = d.weekday || "";
    col.append(pair, wd);
    bars.append(col);
  });

  $("bodyFoot").textContent = "点击 = 单击+右键+中键+侧键，双击折算 1 次 · 滑行距离按 96dpi 估算";
}

// 账单页绑定：翻周 + 设置里的风格分段（切风格用缓存重画，免重拉）
// 洞察翻页交互（billPager）绑定见文件尾部明细翻页器旁，共用冷却逻辑

// ---- 时间线回顾（v1.6.0）：某天 24 小时的前台构成 / 键鼠 / 媒体 ----
let timelineData = null;
let timelineOffset = 0; // 0=今天，正数往过去翻（后端未来封顶今天）
let timelineGeneration = 0;

// 秒 → 紧凑中文时长（时间线行内与汇总用）
function tlDur(secs) {
  if (!secs || secs <= 0) return "0分";
  const h = Math.floor(secs / 3600);
  const m = Math.round((secs % 3600) / 60);
  if (h > 0) return h + "小时" + m + "分";
  if (m > 0) return m + "分";
  return secs + "秒";
}

async function loadDayTimeline() {
  const offset = timelineOffset,
    tab = curBillTab;
  const generation = ++timelineGeneration;
  try {
    const result = await invoke("get_day_timeline", { offset });
    if (generation === timelineGeneration && tab === curBillTab && curView === "viewBill") {
      timelineData = result;
      paintDayTimeline();
    }
  } catch (e) {
    flog("get_day_timeline ERR: " + (e && e.message ? e.message : String(e)));
  }
}

// 行可见性：白天 6–23 点恒显（空行也是时间轴的一部分），凌晨仅在有记录时出现
function tlRowVisible(h) {
  return h.hour >= 6 || h.front_secs > 0 || h.events > 0 || h.audio_secs > 0;
}

// 一行 24% 宽度按秒占比换算：一小时满前台 = 满条，跨小时绝对可比
function tlPct(secs, total) {
  if (!total) return 0;
  return (secs / total) * 100;
}

function paintDayTimeline() {
  const tl = timelineData;
  if (!tl) return;
  $("tlDayLabel").textContent = dateLabel(tl.date);
  $("tlSummary").textContent =
    tl.weekday + " · " + (tl.is_workday ? "工作日" : "休息日") +
    " · 前台 " + tlDur(tl.total_front) +
    " · 键鼠 " + fmtWan(tl.total_events) + " 次";

  const rows = $("tlRows");
  rows.textContent = "";
  const visible = tl.hours.filter(tlRowVisible);
  $("tlEmpty").classList.toggle("hidden", visible.length > 0);
  $("tlNextDay").disabled = timelineOffset <= 0;

  visible.forEach((h) => {
    const row = document.createElement("div");
    row.className = "tl-row";
    const segs = [
      ["tl-seg-work", h.work_secs],
      ["tl-seg-slack", h.slack_secs],
      ["tl-seg-comm", h.comm_secs],
      ["tl-seg-other", h.other_secs],
    ];
    const bar = segs
      .map(([cls, secs]) => {
        const w = tlPct(secs, 3600);
        return w > 0 ? '<i class="' + cls + '" style="width:' + w.toFixed(2) + '%"></i>' : "";
      })
      .join("");
    const note =
      (h.top_app ? escapeHtml(h.top_app) : "无前台") +
      (h.audio_secs > 0 ? " · 在响 " + tlDur(h.audio_secs) : "") +
      " · 键鼠 " + fmtWan(h.events) + " 次";
    row.innerHTML =
      '<span class="tl-hour">' + String(h.hour).padStart(2, "0") + "</span>" +
      '<div class="tl-bar">' + bar + "</div>" +
      '<span class="tl-note">' + note + "</span>";
    row.title =
      h.hour + ":00–" + (h.hour + 1) + ":00 · 前台 " + tlDur(h.front_secs) +
      "（工作 " + tlDur(h.work_secs) + " · 摸鱼 " + tlDur(h.slack_secs) +
      " · 沟通 " + tlDur(h.comm_secs) + " · 其他 " + tlDur(h.other_secs) + "）" +
      " · 键鼠 " + h.events + " 次" +
      (h.audio_secs > 0 ? " · 在响 " + tlDur(h.audio_secs) : "");
    rows.appendChild(row);
  });
}

// 日导航：‹ 往过去翻无上限，› 往未来翻封顶今天
function tlShift(delta) {
  const next = timelineOffset + delta;
  if (next < 0) return;
  timelineOffset = next;
  loadDayTimeline();
}
