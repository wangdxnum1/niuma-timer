const WD_CN = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"]; // Date.getDay() 索引

let curBillTab = "bill";
const BILL_TAB_PANES = {
  bill: "billTabBill",
  report: "billTabReport",
  heat: "billTabHeat",
  trend: "billTabTrend",
  body: "billTabBody",
  timeline: "billTabTimeline",
  focus: "billTabFocus",
};
const BILL_TAB_KEYS = ["bill", "report", "heat", "trend", "body", "timeline", "focus"]; // 翻页器循环顺序
const BILL_TAB_NAMES = {
  bill: "本期账单",
  report: "月报",
  heat: "时段热力",
  trend: "趋势",
  body: "身体账单",
  timeline: "时间线",
  focus: "专注",
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
  // 导出动作只对本周账单 tab 有意义：其余洞察页隐藏工具栏右侧动作区
  $("billActions").classList.toggle("hidden", curBillTab !== "bill");
}

// 懒加载分发：进页 / 切 tab / 翻周才拉对应命令（周级聚合，不进 tick 轮询）
async function loadBillTab() {
  if (curBillTab === "bill") return loadWeekBill();
  if (curBillTab === "report") return loadMonthlyReport();
  if (curBillTab === "heat") return loadHourHeat();
  if (curBillTab === "trend") return loadWeekTrend();
  if (curBillTab === "timeline") return loadDayTimeline();
  if (curBillTab === "focus") return loadFocusSummary();
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
  $("bodyPixels").textContent = fmtDist(body.pixels);
  $("bodyPixelsAvg").textContent = "日均 " + fmtDist(body.pixels / recDays) + "（96dpi 估算）";
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

// 时长文案统一走 core.js 的 fmtDurCN（原本地 tlDur 与 fmtDurCN 措辞漂移："59分钟" vs "60分"）

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
    // 与 monitor/bill 同标准：失败要可见，不再静默空白（此前 tlEmpty 因未渲染保持隐藏，
    // 时间线 tab 永久空白且无文案）。写进行容器 tlRows——成功重绘会先清空它；
    // 不动 tlEmpty，那里的文案语义是「没有记录」，与「加载失败」必须可区分。
    if (generation === timelineGeneration && tab === curBillTab && curView === "viewBill") {
      $("tlRows").innerHTML = '<p class="hint">时间线加载失败（详见 debug.log）</p>';
    }
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
    " · 前台 " + fmtDurCN(tl.total_front) +
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
      (h.audio_secs > 0 ? " · 在响 " + fmtDurCN(h.audio_secs) : "") +
      " · 键鼠 " + fmtWan(h.events) + " 次";
    row.innerHTML =
      '<span class="tl-hour">' + String(h.hour).padStart(2, "0") + "</span>" +
      '<div class="tl-bar">' + bar + "</div>" +
      '<span class="tl-note">' + note + "</span>";
    row.title =
      h.hour + ":00–" + (h.hour + 1) + ":00 · 前台 " + fmtDurCN(h.front_secs) +
      "（工作 " + fmtDurCN(h.work_secs) + " · 摸鱼 " + fmtDurCN(h.slack_secs) +
      " · 沟通 " + fmtDurCN(h.comm_secs) + " · 其他 " + fmtDurCN(h.other_secs) + "）" +
      " · 键鼠 " + h.events + " 次" +
      (h.audio_secs > 0 ? " · 在响 " + fmtDurCN(h.audio_secs) : "");
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

// ---- 专注段聚合（v1.7.0）：周期内每日段数/总时长/最长一段 ----
let focusData = null;
let focusGeneration = 0;

async function loadFocusSummary() {
  const span = curBillSpan, offset = weekOffset, tab = curBillTab;
  const generation = ++focusGeneration;
  try {
    const result = await invoke("get_focus_summary", { span, offset });
    if (generation === focusGeneration && span === curBillSpan && offset === weekOffset && tab === curBillTab && curView === "viewBill") {
      focusData = result;
      paintFocusSummary();
    }
  } catch (e) {
    flog("get_focus_summary ERR: " + (e && e.message ? e.message : String(e)));
    // 同 loadDayTimeline：失败要可见（此前四个指标永远显示「—」无解释）。
    // 写进 focusBars 容器，成功重绘时会被清空重画。
    if (generation === focusGeneration && span === curBillSpan && offset === weekOffset && tab === curBillTab && curView === "viewBill") {
      $("focusBars").innerHTML = '<p class="hint">专注数据加载失败（详见 debug.log）</p>';
    }
  }
}

function paintFocusSummary() {
  const s = focusData;
  if (!s) return;
  const hasData = s.days.length > 0;
  $("focusEmpty").classList.toggle("hidden", hasData);
  $("focusBars").closest("section").classList.toggle("hidden", !hasData);
  if (!hasData) return;

  const recDays = s.days.length;
  $("focusSessions").textContent = String(s.total_sessions);
  $("focusSessionsAvg").textContent = "日均 " + Math.round(s.total_sessions / recDays) + " 段";
  $("focusTotal").textContent = fmtDurCN(s.total_min * 60);
  $("focusTotalAvg").textContent = "日均 " + fmtDurCN(Math.round((s.total_min / recDays) * 60));
  $("focusLongest").textContent = fmtDurCN(s.longest_min * 60);
  // 全周期最长一段的时间标签：取 longest_min 最大的那天的 longest_hm
  const best = s.days.reduce((a, d) => (d.longest_min > (a ? a.longest_min : 0) ? d : a), null);
  $("focusLongestHm").textContent = best && best.longest_hm ? best.longest_hm : "";

  const bd = s.best_day;
  $("focusBestDay").textContent = bd ? bd.date.slice(5).replace("-", ".") + " " + bd.weekday : "—";
  $("focusBestMin").textContent = bd ? "单日 " + fmtDurCN(bd.total_min * 60) : "";

  $("focusFoot").textContent =
    "专注 = 工作类前台 + 键鼠活跃连续达标记一段 · 被打断/离开 5 分钟即断段";

  // 逐日总专注单金柱：只画有专注段的日子（空日本来就没有专注）
  const bars = $("focusBars");
  bars.textContent = "";
  const days = s.days;
  const maxMin = Math.max(1, ...days.map((d) => d.total_min));
  days.forEach((d) => {
    const col = document.createElement("div");
    col.className = "bar-col";
    const pair = document.createElement("div");
    pair.className = "bar-pair";
    const bar = document.createElement("div");
    bar.className = "bar bar-earn";
    bar.style.height = Math.max(3, Math.round((d.total_min / maxMin) * 100)) + "%";
    bar.title = d.date + " " + d.weekday + " · " + d.sessions + " 段 · " +
      fmtDurCN(d.total_min * 60) + (d.longest_hm ? " · 最长 " + d.longest_hm : "");
    pair.append(bar);
    const wd = document.createElement("div");
    wd.className = "bar-wd";
    wd.textContent = d.date.slice(8);
    col.append(pair, wd);
    bars.append(col);
  });
}
