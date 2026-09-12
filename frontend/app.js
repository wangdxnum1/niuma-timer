// 牛马计时器 主界面前端逻辑
// window.__TAURI__ 由 Rust 端 append_invoke_initialization_script 注入的垫片暴露
const TAURI = window.__TAURI__;
const invoke = TAURI.core.invoke;

// 前端版本标记：写进每条日志，用于核对 WebView2 实际加载的是哪个版本（防旧缓存）
const FE_VER = "va4197cb2";

// 主窗口是否可见。托盘常驻期间窗口是 hide 的，此时前端一切轮询都没意义
// （界面看不见，数据看不见），由 Rust 端 1s 线程广播 win-visibility 驱动。
// 初值 true：万一事件系统不通，退化为「始终轮询」的旧行为，不会更差。
let winVisible = true;

// 当前显示的视图 id。主界面与二级页互斥（showView 只留一个 .app 可见），
// 所以「看不见的视图」根本不必重绘——此前每 2 秒会把三个二级页的图表、
// 长列表全量重建一遍，而这些 DOM 正隐藏在主界面背后，纯属浪费。
// 初值 viewMain：万一状态维护失效，退化为「只画主界面」，不会白屏。
let curView = "viewMain";

// 最近一次拉到的监控数据。懒渲染后切进二级页要靠它立即补画，
// 否则得等下一个轮询周期才出内容。
const viewData = { activity: null, appu: null, audio: null };

// 前端调试日志：经 write_debug_log 命令落盘到 %APPDATA%/niuma-timer/debug.log。
// 日志失败自身不抛错，绝不影响主流程。
function flog(msg) {
  try {
    window.__TAURI_INTERNALS__.invoke("write_debug_log", {
      msg: FE_VER + " " + msg,
    }).catch(() => {});
  } catch (_) {}
}

// 全局兜底：未捕获 JS 异常也进日志（否则 WebView2 里用户根本看不到）
window.addEventListener("error", function (ev) {
  flog(
    "JS ERROR: " +
      ev.message +
      " @" +
      (ev.filename || "?") +
      ":" +
      (ev.lineno || "?")
  );
});

const $ = (id) => document.getElementById(id);

async function load() {
  try {
    const cfg = await invoke("load_config");
    $("monthly_salary").value = cfg.monthly_salary;
    $("am_start").value = cfg.am_start;
    $("am_end").value = cfg.am_end;
    $("pm_start").value = cfg.pm_start;
    $("pm_end").value = cfg.pm_end;
    $("payday").value = cfg.payday;
    $("duration_format").value = cfg.duration_format || "hms";
    $("tray_hover_card").checked = !!cfg.tray_hover_card;
    $("tagline_style").value = cfg.tagline_style || "dynamic";
    $("tagline_custom").value = cfg.tagline_custom || "";
    applyTaglineCustomVisibility($("tagline_style").value);
    // 开机自启读注册表真实状态（用户可能在任务管理器手工禁用过），不走 config
    loadAutostart();
    $("workdays_override").value = cfg.workdays_override ?? "";
    lastOverride = cfg.workdays_override ?? null;
    $("overtime_enabled").checked = !!cfg.overtime_enabled;
    applyOvertimeVisibility(cfg.overtime_enabled);
    $("overtime_start").value = cfg.overtime_start || "";
    $("overtime_rate").value = cfg.overtime_rate ?? 20;
    $("overtime_meal_enabled").checked = !!cfg.overtime_meal_enabled;
    $("overtime_meal").value = cfg.overtime_meal ?? 20;
    $("weekend_overtime").checked = !!cfg.weekend_overtime;
    $("weekend_ot_start").value = cfg.weekend_ot_start || "";
    $("overtime_rate_weekend").value = cfg.overtime_rate_weekend ?? "";
    $("overtime_rate_holiday").value = cfg.overtime_rate_holiday ?? "";
    applyRestOvertimeVisibility(cfg.weekend_overtime);
    $("app_whitelist_enabled").checked = !!cfg.app_whitelist_enabled;
    renderWhitelist(cfg.app_whitelist || []);
    $("monitor_activity").checked = cfg.monitor_activity !== false;
    $("monitor_app_usage").checked = cfg.monitor_app_usage !== false;
    $("monitor_audio").checked = cfg.monitor_audio !== false;
    syncMonitorState();
    $("retention_days").value = String(cfg.retention_days || 0);
    loadStorageInfo();
    // 初始快照：与 readCfg() 字段顺序一致，用于失焦保存时判断是否有变化
    lastSaved = JSON.stringify(readCfg());
  } catch (e) {
    flog("load_config ERR: " + (e && e.message ? e.message : String(e)));
    console.error(e);
  }
}

// ---- 自动保存（控件失去焦点时触发）----
let lastSaved = null; // 上次成功保存的配置 JSON 快照，用于去重
let lastOverride = null; // 上次保存的上班天数，用于判断是否需静默刷新工作日数据

// 三个监控开关的内存状态（同步自配置；关闭时对应卡片显示停用提示并跳过轮询）
let monitors = { activity: true, app_usage: true, audio: true };
function syncMonitorState() {
  monitors.activity = $("monitor_activity").checked;
  monitors.app_usage = $("monitor_app_usage").checked;
  monitors.audio = $("monitor_audio").checked;
}

// ---- 应用使用白名单（设置页可维护）----
// 从 DOM 列表读取当前白名单（去重、去空、大小写规范化仅用于判等，展示名原样保留）
function readWhitelist() {
  const out = [];
  const seen = new Set();
  document.querySelectorAll("#appWhitelistList .wl-chip-text").forEach((el) => {
    const v = (el.textContent || "").trim();
    const k = v.toLowerCase();
    if (v && !seen.has(k)) {
      seen.add(k);
      out.push(v);
    }
  });
  return out;
}

// 渲染白名单列表为可删除 chip；空列表显示提示
function renderWhitelist(list) {
  const box = $("appWhitelistList");
  if (!box) return;
  box.innerHTML = "";
  const seen = new Set();
  const items = (list || [])
    .map((x) => (x || "").trim())
    .filter((x) => {
      const k = x.toLowerCase();
      if (!x || seen.has(k)) return false;
      seen.add(k);
      return true;
    });
  if (items.length === 0) {
    box.innerHTML = '<p class="wl-empty">名单为空：当前统计全部应用</p>';
    return;
  }
  for (const name of items) box.appendChild(makeChip(name));
}

function makeChip(name) {
  const chip = document.createElement("span");
  chip.className = "wl-chip";
  const txt = document.createElement("span");
  txt.className = "wl-chip-text";
  txt.textContent = name;
  const rm = document.createElement("button");
  rm.className = "wl-rm";
  rm.textContent = "×";
  rm.title = "移除";
  rm.addEventListener("click", () => {
    chip.remove();
    if ($("appWhitelistList").children.length === 0) renderWhitelist([]);
    saveNow();
  });
  chip.appendChild(txt);
  chip.appendChild(rm);
  return chip;
}

function addWhitelistItem() {
  const input = $("appWhitelistInput");
  const v = (input.value || "").trim();
  if (!v) return;
  const k = v.toLowerCase();
  let dup = false;
  document.querySelectorAll("#appWhitelistList .wl-chip-text").forEach((el) => {
    if ((el.textContent || "").trim().toLowerCase() === k) dup = true;
  });
  if (dup) {
    input.value = "";
    return;
  }
  const box = $("appWhitelistList");
  if (box.querySelector(".wl-empty")) box.innerHTML = "";
  box.appendChild(makeChip(v));
  input.value = "";
  input.focus();
  saveNow();
}

function currentYearMonth() {
  const d = new Date();
  const m = d.getMonth() + 1;
  return d.getFullYear() + "-" + (m < 10 ? "0" + m : "" + m);
}

function readCfg() {
  const salaryRaw = $("monthly_salary").value.trim();
  const paydayRaw = $("payday").value.trim();
  const overrideRaw = $("workdays_override").value.trim();
  const cfg = {
    am_start: $("am_start").value,
    am_end: $("am_end").value,
    pm_start: $("pm_start").value,
    pm_end: $("pm_end").value,
    duration_format: $("duration_format").value || "hms",
    tray_hover_card: $("tray_hover_card").checked,
    tagline_style: $("tagline_style").value || "dynamic",
    tagline_custom: $("tagline_custom").value || "",
    overtime_enabled: $("overtime_enabled").checked,
    overtime_start: $("overtime_start").value || null,
    overtime_rate: parseFloat($("overtime_rate").value) || 0,
    overtime_meal_enabled: $("overtime_meal_enabled").checked,
    overtime_meal: parseFloat($("overtime_meal").value) || 0,
    monitor_activity: $("monitor_activity").checked,
    monitor_app_usage: $("monitor_app_usage").checked,
    monitor_audio: $("monitor_audio").checked,
    weekend_overtime: $("weekend_overtime").checked,
    weekend_ot_start: $("weekend_ot_start").value || null,
    overtime_rate_weekend: numOrNull($("overtime_rate_weekend").value),
    overtime_rate_holiday: numOrNull($("overtime_rate_holiday").value),
    app_whitelist_enabled: $("app_whitelist_enabled").checked,
    app_whitelist: readWhitelist(),
    retention_days: parseInt($("retention_days").value) || 0,
  };
  // 月薪/发薪日留空：不传该字段，后端合并时保留旧值，避免误存 0/1，也不挡住其它开关保存
  if (salaryRaw !== "") cfg.monthly_salary = parseFloat(salaryRaw) || 0;
  if (paydayRaw !== "") cfg.payday = parseInt(paydayRaw) || 1;
  if (overrideRaw) {
    cfg.workdays_override = parseInt(overrideRaw);
    cfg.workdays_override_for = currentYearMonth();
  } else {
    cfg.workdays_override = null;
    cfg.workdays_override_for = null;
  }
  return cfg;
}

// 数字输入：留空返回 null（表示沿用上一级费率），有值才解析
function numOrNull(v) {
  const s = String(v == null ? "" : v).trim();
  if (s === "") return null;
  const n = parseFloat(s);
  return isNaN(n) ? null : n;
}

// 控件失焦时调用：配置无变化则不写盘（去重）
function saveIfChanged() {
  if (JSON.stringify(readCfg()) === lastSaved) return;
  doSave();
}

// 开关等明确变更：直接保存
function saveNow() {
  doSave();
}

async function doSave() {
  const cfg = readCfg();
  try {
    await invoke("save_config", { cfg });
    lastSaved = JSON.stringify(cfg);
    showToast("已自动保存", "ok");
    // 仅当上班天数被修改时才静默刷新工作日数据（避免每次保存都发网络请求）
    if (cfg.workdays_override !== lastOverride) {
      lastOverride = cfg.workdays_override;
      silentRefresh();
    }
  } catch (e) {
    showToast("保存失败：" + e, "err");
    // lastSaved 未更新：下次失焦会自动重试
  }
}

// toast：右下角气泡，连续编辑只重置计时不重播动画
let toastTimer = null;
function showToast(msg, type) {
  const t = $("toast");
  t.textContent = msg;
  t.className = "toast show " + type;
  clearTimeout(toastTimer);
  const dur = type === "err" ? 3000 : 1600;
  toastTimer = setTimeout(() => {
    t.className = "toast " + type;
  }, dur);
}

async function refresh() {
  try {
    const n = await invoke("refresh_holidays");
    $("workdaysInfo").textContent = "当月实际上班天数：" + n + " 天";
  } catch (e) {
    $("workdaysInfo").textContent = "刷新失败：" + e;
  }
}

// 静默刷新：自动保存触发，失败不打扰用户
async function silentRefresh() {
  try {
    const n = await invoke("refresh_holidays");
    $("workdaysInfo").textContent = "当月实际上班天数：" + n + " 天";
  } catch (e) {
    /* 静默失败，稍后可手动刷新 */
  }
}

// 最近一次拿到的状态。改副标题设置时要立刻重画，靠它免掉一次多余 IPC。
let lastStatus = null;

async function tick() {
  try {
    const s = await invoke("get_status_cmd");
    lastStatus = s;
    $("earned").textContent = "¥" + s.earned.toFixed(2);
    renderBadge(s);
    renderSparkline(s);
    renderTimeline(s);
    // 赚钱进度条：已赚 / 全天应赚（时薪 × 当日总工时，DayStatus 现成字段）。
    // 休息日或时薪为 0 时整块隐藏；已下班封顶 100%（加班费不进此条）
    const liveProgress = $("liveProgress");
    if (s.is_workday && s.hourly_rate > 0 && s.daily_hours > 0) {
      const target = s.hourly_rate * s.daily_hours;
      const pct = Math.min(100, (s.earned / target) * 100);
      liveProgress.classList.remove("hidden");
      $("lpFill").style.width = pct.toFixed(1) + "%";
      $("lpTarget").textContent = "¥" + target.toFixed(2);
      $("lpPct").textContent = Math.round(pct) + "%";
      $("lpMeta").textContent =
        "速率 ¥" + s.rate_per_min.toFixed(2) + "/分 · 距发薪 " + s.days_to_pay + " 天";
    } else {
      liveProgress.classList.add("hidden");
    }
    renderTagline(s);
  } catch (e) {
    /* 忽略瞬时错误 */
  }
}

// ---- 状态徽章 / 赚钱走势 / 今日时间轴 ----

// 距下班的小时数 → 紧凑文案（徽章空间有限，不用 hms 全格式）
function fmtShortH(h) {
  return h < 1 ? Math.max(1, Math.round(h * 60)) + " 分钟" : h.toFixed(1) + "h";
}

// 状态徽章：主页品牌行右侧的动态状态（搬砖中 / 已下班 / 今天休息）
function renderBadge(s) {
  const badge = $("statusBadge");
  if (!s.is_workday) {
    badge.textContent = "今天休息";
    badge.className = "badge off";
  } else if (s.off_work) {
    badge.textContent = "已下班 · 辛苦了";
    badge.className = "badge off";
  } else {
    badge.textContent = "● 搬砖中 · 距下班 " + fmtShortH(s.to_off_h);
    badge.className = "badge";
  }
}

// 落在 [s,e] 时段内的分钟数（与后端 calc::overlap 同口径，只服务本文件可视化）
function overlapMin(t, s, e) {
  return t <= s ? 0 : t >= e ? e - s : t - s;
}

// 赚钱走势：以配置时段 + 当前时薪重建「今日已赚」曲线（0 时 → 现在，15 分钟步长）。
// 已赚随时段的函数是确定的（分段线性），因此无需新命令、无需存储——
// 曲线画的是事实而非预测，未到的时刻不画
function renderSparkline(s) {
  const box = $("earnedSpark");
  const axis = $("sparkAxis");
  const amS = minutesOf($("am_start").value);
  const amE = minutesOf($("am_end").value);
  const pmS = minutesOf($("pm_start").value);
  const pmE = minutesOf($("pm_end").value);
  if (
    !s.is_workday ||
    s.hourly_rate <= 0 ||
    amS === null ||
    amE === null ||
    pmS === null ||
    pmE === null
  ) {
    box.classList.add("hidden");
    axis.classList.add("hidden");
    return;
  }
  box.classList.remove("hidden");
  axis.classList.remove("hidden");
  const now = new Date();
  const nowMin = now.getHours() * 60 + now.getMinutes();
  const perMin = s.hourly_rate / 60;
  const H = 40; // 与 svg viewBox 高度一致
  const max = s.hourly_rate * s.daily_hours; // 满幅 = 全天应赚
  const ts = [];
  for (let t = 0; t < nowMin; t += 15) ts.push(t);
  ts.push(nowMin);
  const path = ts
    .map((t, i) => {
      const v = (overlapMin(t, amS, amE) + overlapMin(t, pmS, pmE)) * perMin;
      const x = (t / 1440) * 440;
      const y = H - 1 - (max > 0 ? (v / max) * (H - 3) : 0);
      return (i ? "L" : "M") + x.toFixed(1) + "," + y.toFixed(1);
    })
    .join(" ");
  $("sparkLine").setAttribute("d", path);
  $("sparkFill").setAttribute(
    "d",
    path + " L" + ((nowMin / 1440) * 440).toFixed(1) + "," + H + " L0," + H + " Z"
  );
}

// 今日时间轴：上班/午休/下班按配置时间分段，白色「现在」指针标出当前位置。
// 宽度全部按配置时段比例计算，改配置立即生效；休息日整块隐藏
function renderTimeline(s) {
  const box = $("dayTimeline");
  const amS = minutesOf($("am_start").value);
  const amE = minutesOf($("am_end").value);
  const pmS = minutesOf($("pm_start").value);
  const pmE = minutesOf($("pm_end").value);
  if (
    !s.is_workday ||
    amS === null ||
    amE === null ||
    pmS === null ||
    pmE === null ||
    pmE <= amS
  ) {
    box.classList.add("hidden");
    return;
  }
  box.classList.remove("hidden");
  const span = pmE - amS;
  const pct = (v) => (Math.min(Math.max(v, 0), span) / span) * 100 + "%";
  const now = new Date();
  const nowMin = now.getHours() * 60 + now.getMinutes();
  const amWorked = overlapMin(nowMin, amS, amE);
  const pmWorked = overlapMin(nowMin, pmS, pmE);
  $("tlAmDone").style.width = pct(amWorked);
  $("tlAmTodo").style.width = pct(amE - amS - amWorked);
  $("tlRest").style.width = pct(pmS - amE);
  $("tlPmDone").style.width = pct(pmWorked);
  $("tlPmTodo").style.width = pct(pmE - pmS - pmWorked);
  $("tlNow").style.left = pct(Math.min(Math.max(nowMin, amS), pmE));
  $("tlAmStart").textContent = $("am_start").value;
  $("tlAmEnd").textContent = $("am_end").value;
  $("tlPmStart").textContent = $("pm_start").value;
  $("tlPmEnd").textContent = $("pm_end").value;
}

// ---- 首页副标题 ----

// 固定文案表。key 与设置页下拉的 value 一一对应
const FIXED_TAGLINES = {
  price: "你今天的每一分钟，都明码标价",
  rise: "每一秒，钱都在涨",
  count: "搬砖的每一分钟，都算数",
  classic: "实时计算你今天赚了多少钱",
};

function taglineStyle() {
  const el = $("tagline_style");
  const v = el ? el.value : "";
  return v || "dynamic"; // 控件还没填好时退回动态，不显示空白
}

// "HH:MM" -> 当日分钟数；空值或格式不合法返回 null。
// 不能用 0 兜底：否则「上午下班 00:00」这类合法值会被当成无效而跳过午休判断。
function minutesOf(hhmm) {
  const p = String(hhmm || "").split(":");
  if (p.length !== 2) return null;
  const h = parseInt(p[0], 10);
  const m = parseInt(p[1], 10);
  if (isNaN(h) || isNaN(m)) return null;
  return h * 60 + m;
}

// 动态副标题：把「实时」这个卖点用起来，而不是写死一句说明文。
// 全部基于 tick 已拿到的状态，不发额外 IPC。
function dynamicTagline(s) {
  if (s.to_off_str === "今天休息") return "今天休息，钱也休息";
  if (s.off_work) return "今天的钱，就到这儿了";
  if (s.worked_h <= 0) return "还没开工，钱暂时没动";
  // 午休：当前时刻落在上午下班与下午上班之间
  const now = new Date().getHours() * 60 + new Date().getMinutes();
  const amEnd = minutesOf($("am_end").value);
  const pmStart = minutesOf($("pm_start").value);
  if (amEnd !== null && pmStart !== null && pmStart > amEnd && now >= amEnd && now < pmStart) {
    return "午休中，钱先歇会儿";
  }
  // 临近下班：剩 30 分钟以内，报具体分钟数
  if (s.to_off_h > 0 && s.to_off_h <= 0.5) {
    return "还有 " + Math.max(1, Math.round(s.to_off_h * 60)) + " 分钟，撑住";
  }
  return "正在搬砖，钱一直在涨";
}

function renderTagline(s) {
  const el = $("tagline");
  if (!el) return;
  const style = taglineStyle();
  if (style === "none") {
    el.style.display = "none";
    return;
  }
  el.style.display = "";
  if (style === "custom") {
    // 自定义为空时退回动态句，不把界面留白
    const t = ($("tagline_custom").value || "").trim();
    el.textContent = t || dynamicTagline(s);
    return;
  }
  if (style === "dynamic") {
    el.textContent = dynamicTagline(s);
    return;
  }
  el.textContent = FIXED_TAGLINES[style] || dynamicTagline(s);
}

// 只有选了「自定义」才露出输入框
function applyTaglineCustomVisibility(v) {
  const row = $("taglineCustomRow");
  if (row) row.style.display = v === "custom" ? "" : "none";
}

// 加班开关关闭时隐藏主界面「加班总览」卡片；已有数据保留在库中不受影响
function applyOvertimeVisibility(enabled) {
  const card = $("otCard");
  if (card) card.style.display = enabled ? "" : "none";
}

// 休息日/节假日加班关闭时隐藏其三项子配置，避免看到一堆不生效的输入框
function applyRestOvertimeVisibility(enabled) {
  const box = $("restOtFields");
  if (box) box.style.display = enabled ? "" : "none";
}

// 加班明细当前查看的年月；null = 跟随当月
let otView = null;

// 加班记录加载与渲染
async function loadOvertime() {
  // 加班追踪关闭时不拉取、不展示（历史记录仍保留在 SQLite，开关不影响数据）
  if (!$("overtime_enabled").checked) return;
  const now = new Date();
  const curY = now.getFullYear();
  const curM = now.getMonth() + 1;
  const vy = otView ? otView.year : curY;
  const vm = otView ? otView.month : curM;
  try {
    const ot = await invoke("get_overtime_records", { year: vy, month: vm });
    renderOtTable(ot);
    // 主界面「加班总览」固定反映当月：翻到历史月份时要单独拉当月，
    // 否则主界面会显示 8 月的合计却标着「本月合计」，属于数据错位。
    if (vy === curY && vm === curM) {
      renderOtHome(ot);
    } else {
      const home = await invoke("get_overtime_records", { year: curY, month: curM });
      renderOtHome(home);
    }
  } catch (e) {
    console.error("loadOvertime error:", e);
  }
}

// 翻月：delta = -1 上一月 / +1 下一月
function shiftOtMonth(delta) {
  const now = new Date();
  const base = otView || { year: now.getFullYear(), month: now.getMonth() + 1 };
  let y = base.year;
  let m = base.month + delta;
  if (m < 1) {
    m = 12;
    y -= 1;
  } else if (m > 12) {
    m = 1;
    y += 1;
  }
  otView = { year: y, month: m };
  loadOvertime();
}

// 主界面「本月加班战果」金色卡：只吃当月数据（¥ 符号在模板里，这里只写数值）
function renderOtHome(ot) {
  $("otMonthTotal").textContent = ot.total_all.toFixed(0);
  $("otMonthHours").textContent = ot.total_hours.toFixed(1) + "h";
  $("otMonthDays").textContent = ot.days + " 天";
  $("otMonthAvg").textContent =
    "¥" + (ot.days > 0 ? (ot.total_all / ot.days).toFixed(0) : "0");
}

// 加班明细页：表格 + 所选月份小计，跟随 otView
function renderOtTable(ot) {
  $("otMonthLabel").textContent = ot.year + " 年 " + ot.month + " 月";
  const sum = $("otMonthSummary");
  if (sum) {
    sum.textContent = ot.records.length
      ? "合计 ¥" + ot.total_all.toFixed(0) +
        " · " + ot.total_hours.toFixed(1) + " 小时 · " + ot.days + " 天"
      : "本月暂无加班记录";
  }
  // 下一月按钮翻到当前月为止：未来没有加班记录
  const now = new Date();
  const curY = now.getFullYear();
  const curM = now.getMonth() + 1;
  $("otNextMonth").disabled =
    ot.year > curY || (ot.year === curY && ot.month >= curM);
  const tbody = $("ot_tbody");
  tbody.innerHTML = "";
  if (ot.records.length === 0) {
    tbody.innerHTML =
      '<tr><td colspan="8" class="ot-empty">暂无加班记录</td></tr>';
    return;
  }
  for (const r of [...ot.records].reverse()) {
    const tr = document.createElement("tr");
    // 来源：1 = 手动录入（不会被自动锁屏记录覆盖），0 = 自动
    const isManual = r.source === 1;
    const cells = [
      r.date.slice(5),
      // 跨午夜的离开时刻是次日凌晨，光看 "01:30" 会被误读成当天凌晨
      r.cross_midnight ? "次日 " + r.lock_time : r.lock_time,
      r.valid_hours.toFixed(1) + "h",
      "¥" + r.fee.toFixed(0),
      r.meal > 0 ? "¥" + r.meal.toFixed(0) : "—",
      "¥" + r.total.toFixed(0),
      isManual ? "手动" : "自动",
    ];
    cells.forEach((c, i) => {
      const td = document.createElement("td");
      td.textContent = c;
      if (i === cells.length - 1) {
        td.className = isManual ? "ot-src manual" : "ot-src";
      }
      tr.appendChild(td);
    });
    const opTd = document.createElement("td");
    opTd.className = "ot-op";
    const editBtn = document.createElement("button");
    editBtn.textContent = "编辑";
    editBtn.className = "btn-sm";
    editBtn.onclick = () => editOtRecord(r);
    opTd.appendChild(editBtn);
    const delBtn = document.createElement("button");
    delBtn.textContent = "删除";
    delBtn.className = "btn-sm danger";
    delBtn.onclick = () => deleteOtRecord(r.date);
    opTd.appendChild(delBtn);
    tr.appendChild(opTd);
    tbody.appendChild(tr);
  }
}

// ---- 加班记录手动增删改（仅当月）----
function showOtForm() {
  $("otForm").classList.remove("hidden");
}
function hideOtForm() {
  $("otForm").classList.add("hidden");
  $("otfMsg").textContent = "";
}
function showOtMsg(msg) {
  $("otfMsg").textContent = msg;
}

// 点击「添加记录」：自动预填今天日期 + 当前时刻，焦点落在下班时间上
function openOtForm() {
  const d = new Date();
  const pad = (n) => String(n).padStart(2, "0");
  $("otf_date").value =
    d.getFullYear() + "-" + pad(d.getMonth() + 1) + "-" + pad(d.getDate());
  $("otf_lock").value = pad(d.getHours()) + ":" + pad(d.getMinutes());
  $("otf_start").value = "";
  $("otf_cross").checked = false;
  $("otfTitle").textContent = "添加加班记录";
  $("otfSave").textContent = "保存";
  showOtMsg("");
  showOtForm();
  $("otf_lock").focus();
}

// 点击某行的「编辑」
function editOtRecord(r) {
  $("otf_date").value = r.date;
  $("otf_lock").value = r.lock_time;
  $("otf_start").value = r.ot_start && r.ot_start !== "" ? r.ot_start : "";
  $("otf_cross").checked = !!r.cross_midnight;
  $("otfTitle").textContent = "编辑加班记录";
  $("otfSave").textContent = "更新";
  showOtMsg("");
  showOtForm();
  $("otf_lock").focus();
}

// 提交添加/更新（后端按日期 upsert，同一日期即覆盖=编辑）
async function submitOtForm() {
  const date = $("otf_date").value;
  const lock = $("otf_lock").value;
  const start = $("otf_start").value || null;
  // 嵌套 input 按 serde 原样反序列化（只有顶层命令参数才转 camelCase），
  // 字段必须与 ManualOvertimeInput 一致：写成 crossMidnight 会被丢掉，永远是 false。
  const cross_midnight = $("otf_cross").checked;
  if (!date || !lock) {
    showOtMsg("请填写日期和下班时间");
    return;
  }
  // 前端先把关：不能是未来日期。历史月份允许补录——界面已能切月查看，
  // 再把写入锁死在当月就没法补记忘掉的加班了。
  const picked = new Date(date + "T00:00:00");
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  if (picked > today) {
    showOtMsg("不能添加未来日期的加班记录");
    return;
  }
  try {
    const view = await invoke("save_overtime_record", {
      input: { date, lock_time: lock, ot_start: start, cross_midnight },
    });
    // 跟到记录所属月份：补录 8 月时视图停在 8 月，不会莫名跳回当月
    otView = { year: view.year, month: view.month };
    hideOtForm();
    showToast("已保存加班记录", "ok");
    loadOvertime();
  } catch (e) {
    showOtMsg("保存失败：" + e);
  }
}

// 自定义确认弹窗（替代浏览器默认 confirm）
let confirmResolve = null;
function showConfirm(text) {
  return new Promise((resolve) => {
    confirmResolve = resolve;
    $("confirmText").textContent = text;
    $("confirmModal").classList.remove("hidden");
  });
}
function hideConfirm(result) {
  $("confirmModal").classList.add("hidden");
  if (confirmResolve) {
    confirmResolve(result);
    confirmResolve = null;
  }
}
$("confirmOk").addEventListener("click", () => hideConfirm(true));
$("confirmCancel").addEventListener("click", () => hideConfirm(false));

// 删除某天记录
async function deleteOtRecord(date) {
  const ok = await showConfirm("确定删除 " + date + " 的加班记录？");
  if (!ok) return;
  try {
    const view = await invoke("delete_overtime_record", { date });
    otView = { year: view.year, month: view.month };
    loadOvertime();
    showToast("已删除", "ok");
  } catch (e) {
    showToast("删除失败：" + e, "err");
  }
}

// ---- 活动统计（鼠标/键盘）----

// 像素 → 可读距离：96dpi 下 1 英寸 = 96px，1 米 ≈ 3779.5px
function fmtDist(px) {
  if (px == null || px < 1000) return (px || 0) + "px";
  const m = px / 3779.5;
  if (m < 1000) return m.toFixed(0) + "m";
  return (m / 1000).toFixed(2) + "km";
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  }[c]));
}

function bucketEvents(b) {
  if (!b) return 0;
  return (b.moves || 0) + (b.left || 0) + (b.dbl || 0) + (b.right || 0) +
    (b.wheel || 0) + (b.mid || 0) + (b.xbtn || 0) + (b.keys || 0);
}

// 某小时桶的简要描述（tooltip 用）
function fmtBucket(b, i) {
  const parts = [];
  if (b.moves) parts.push("移动 " + b.moves.toLocaleString());
  if (b.left) parts.push("左键 " + b.left.toLocaleString());
  if (b.dbl) parts.push("双击 " + b.dbl.toLocaleString());
  if (b.right) parts.push("右键 " + b.right.toLocaleString());
  if (b.mid) parts.push("中键 " + b.mid.toLocaleString());
  if (b.xbtn) parts.push("侧键 " + b.xbtn.toLocaleString());
  if (b.keys) parts.push("按键 " + b.keys.toLocaleString());
  if (b.wheel) parts.push("滚轮 " + b.wheel + " 次");
  return i + "时 · " + (parts.length ? parts.join("、") : "无活动");
}

// ---- 历史日期查看（活动 / 应用使用 / 媒体播放共用）----
// null = 跟随今天。翻到历史日期时只影响对应二级页：主界面「今日」卡片由各 paint
// 分支用 isToday 挡住，不会被历史数据顶掉；返回主界面时统一复位。
const histDate = { act: null, appu: null, audio: null };

function todayStr() {
  const d = new Date();
  return fmtYMD(d.getFullYear(), d.getMonth() + 1, d.getDate());
}

function fmtYMD(y, m, d) {
  const p = (n) => (n < 10 ? "0" + n : "" + n);
  return y + "-" + p(m) + "-" + p(d);
}

// "YYYY-MM-DD" 加减天数。用本地时间构造 Date，避免 UTC 解析把日期串到前一天
function addDays(s, delta) {
  const p = String(s || "").split("-").map(Number);
  if (p.length !== 3 || p.some(isNaN)) return todayStr();
  const d = new Date(p[0], p[1] - 1, p[2]);
  d.setDate(d.getDate() + delta);
  return fmtYMD(d.getFullYear(), d.getMonth() + 1, d.getDate());
}

function isToday(s) {
  return !!s && s === todayStr();
}

// 导航条上的日期：今天 / 昨天 / 9月8日（跨年补年份）
function dateLabel(s) {
  if (!s || isToday(s)) return "今天";
  if (s === addDays(todayStr(), -1)) return "昨天";
  const p = String(s).split("-").map(Number);
  const now = new Date();
  return p[0] !== now.getFullYear()
    ? p[0] + "年" + p[1] + "月" + p[2] + "日"
    : p[1] + "月" + p[2] + "日";
}

// 二级页标题：今日活动明细 / 昨日活动明细 / 9月8日活动明细
function dayTitle(s, name) {
  if (!s || isToday(s)) return "今日" + name;
  if (s === addDays(todayStr(), -1)) return "昨日" + name;
  return dateLabel(s) + name;
}

// 空态文案：历史日期不能再说「今日暂无…」
function dayEmpty(s, what) {
  return isToday(s) ? "今日暂无" + what : dateLabel(s) + "暂无" + what;
}

// 翻天：delta = -1 前一天 / +1 后一天。未来没有数据，翻不过去
function shiftHist(key, delta, loadFn) {
  const next = addDays(histDate[key] || todayStr(), delta);
  if (next > todayStr()) return; // "YYYY-MM-DD" 定长，字典序即时间序
  histDate[key] = next;
  updateDayNav(key);
  loadFn();
}

// 导航条日期与「后一天」可用状态
const DAY_NAV = {
  act: { label: "actDayLabel", next: "actNextDay", title: "actTitle", name: "活动明细" },
  appu: { label: "appuDayLabel", next: "appuNextDay", title: "appuTitle", name: "应用使用" },
  audio: { label: "audioDayLabel", next: "audioNextDay", title: "audioTitle", name: "媒体播放" },
};

function updateDayNav(key) {
  const cfg = DAY_NAV[key];
  if (!cfg) return;
  const d = histDate[key];
  const lb = $(cfg.label);
  const nx = $(cfg.next);
  const ti = $(cfg.title);
  if (lb) lb.textContent = dateLabel(d);
  if (nx) nx.disabled = isToday(d || todayStr());
  if (ti) ti.textContent = dayTitle(d, cfg.name);
}

// 返回主界面时把历史日期复位：否则缓存里留着历史数据，
// 主界面「今日」卡片会被 paint 的 isToday 守卫挡住而停在旧数字上
function resetHistDates() {
  let dirty = false;
  const viewKey = { act: "activity", appu: "appu", audio: "audio" };
  for (const k of Object.keys(viewKey)) {
    if (histDate[k] !== null) {
      histDate[k] = null;
      viewData[viewKey[k]] = null; // 作废历史缓存，repaintCurrentView 会现拉今天的
      dirty = true;
    }
  }
  if (dirty) {
    for (const k of Object.keys(DAY_NAV)) updateDayNav(k);
  }
}

async function loadActivity() {
  if (!monitors.activity) {
    // 停用即清缓存：否则从设置页切回主界面时，会拿旧数据画出已停用模块的数字
    viewData.activity = null;
    // 已停用：主界面卡片显示占位符
    if (curView === "viewMain") {
      ["act_left", "act_right", "act_keys", "act_hours"].forEach(
        (id) => ($(id).textContent = "—")
      );
    }
    return;
  }
  try {
    const a = await invoke("get_activity_summary", { date: histDate.act });
    viewData.activity = a;
    paintActivity();
  } catch (e) {
    console.error("loadActivity error:", e);
  }
}

// 只画当前看得见的那部分：主界面画 4 个汇总数字，活动明细页画图表与明细。
// 其余视图（加班/应用/媒体/设置）下活动相关 DOM 全不可见，直接跳过重绘。
function paintActivity() {
  const a = viewData.activity;
  if (!a) return;
  const t = a.totals || {};
  if (curView === "viewMain") {
    // 主界面卡片固定反映今天：翻到历史日期后缓存里是历史数据，不能拿去覆盖「今日」
    if (!isToday(a.date)) return;
    // 单击次数 = 左键按下总数 − 双击的两连按（双击一次含 2 次按下，activity 后端如此计数）
    const dbl = t.dbl || 0;
    $("act_left").textContent = Math.max(0, (t.left || 0) - 2 * dbl).toLocaleString();
    $("act_right").textContent = (t.right || 0).toLocaleString();
    $("act_keys").textContent = (t.keys || 0).toLocaleString();
    $("act_hours").textContent = (a.active_hours || 0) + "h";
    // 辛苦数据条（Weather 式一行四格）：左键 / 右键 / 敲击 / 活跃时段
    $("stLeft").textContent = (t.left || 0).toLocaleString();
    $("stRight").textContent = (t.right || 0).toLocaleString();
    $("stKeys").textContent = (t.keys || 0).toLocaleString();
    $("stHours").textContent = (a.active_hours || 0) + "h";
    // 监控卡活动面板（8 格）：双击 / 滚轮 / 移动次数 / 移动距离为新增项
    $("monDbl").textContent = (t.dbl || 0).toLocaleString();
    $("monWheel").textContent =
      (t.wheel || 0).toLocaleString() + " 次 · " + (t.wheel_ticks || 0).toLocaleString() + " 格";
    $("monMoves").textContent = (t.moves || 0).toLocaleString();
    $("monDist").textContent = fmtDist(t.pixels);
    return;
  }
  if (curView !== "viewAct") return;
  updateDayNav("act");
  $("actL").textContent = (t.left || 0).toLocaleString();
  $("actD").textContent = (t.dbl || 0).toLocaleString();
  $("actR").textContent = (t.right || 0).toLocaleString();
  $("actW").textContent = (t.wheel || 0) + " 次 · " + (t.wheel_ticks || 0) + " 格";
  $("actM").textContent = (t.moves || 0).toLocaleString();
  $("actK").textContent = (t.keys || 0).toLocaleString();
  $("actP").textContent = fmtDist(t.pixels);
  $("actH").textContent = (a.active_hours || 0) + " 小时";
  renderChart(a.hourly || [], isToday(a.date));
  renderTopKeys(a.top_keys || []);
}

// 逐小时活跃柱状图（纯 CSS，无第三方库）
// isToday=false 时不标「当前小时」——历史日期没有当前小时，标金色属于误导
function renderChart(hourly, isToday = true) {
  const el = $("actChart");
  if (!el) return;
  const now = isToday ? new Date().getHours() : -1;
  const max = Math.max(1, ...hourly.map((b) => bucketEvents(b)));
  el.innerHTML = "";
  hourly.forEach((b, i) => {
    const v = bucketEvents(b);
    const hgt = v > 0 ? Math.max(5, Math.round((v / max) * 100)) : 2;
    const d = document.createElement("div");
    d.className = "bar" + (v > 0 ? "" : " empty") + (i === now ? " cur" : "");
    d.style.height = hgt + "%";
    d.title = fmtBucket(b, i);
    el.appendChild(d);
  });
}

// 高频按键 Top 榜
function renderTopKeys(list) {
  const el = $("actTopKeys");
  if (!el) return;
  el.innerHTML = "";
  if (!list.length) {
    el.innerHTML = '<p class="hint">暂无按键数据</p>';
    return;
  }
  const max = list[0].count;
  for (const k of list) {
    const row = document.createElement("div");
    row.className = "tk-row";
    row.innerHTML =
      '<span class="tk-key">' + escapeHtml(k.key) + "</span>" +
      '<div class="tk-bar"><div style="width:' + Math.round((k.count / max) * 100) + '%"></div></div>' +
      '<span class="tk-count">' + k.count.toLocaleString() + "</span>";
    el.appendChild(row);
  }
}

// ---- 应用使用时长 ----

// 秒数 → "x小时x分 / x分钟 / x秒"
function fmtDurCN(sec) {
  sec = Math.max(0, Math.round(sec || 0));
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  if (h > 0) return h + "小时" + (m > 0 ? m + "分" : "");
  if (m > 0) return m + "分钟";
  return sec + "秒";
}

async function loadAppUsage() {
  if (!monitors.app_usage) {
    viewData.appu = null; // 同上，停用即清缓存
    if (curView === "viewMain") {
      showCardHint("appuHomeList", "已在设置中关闭应用使用监控");
    }
    return;
  }
  try {
    // 参数名用 camelCase：Tauri v2 的命令宏会把 Rust 的 snake_case 参数统一转成 camelCase
    const s = await invoke("get_app_usage_summary", {
      knownIcons: knownIcons(),
      date: histDate.appu,
    });
    mergeIcons(s.apps || []); // 图标入缓存与当前视图无关，必须在这里做
    viewData.appu = s;
    paintAppUsage();
    if (audioLogged.appu < 2) {
      audioLogged.appu++;
      const n = s.apps ? s.apps.length : "?";
      flog("appu ok #" + audioLogged.appu + ": apps=" + n);
    }
  } catch (e) {
    flog("appu ERR: " + (e && e.message ? e.message : String(e)));
    showCardError("appuHomeList", "应用使用数据加载失败");
    console.error("loadAppUsage error:", e);
  }
}

// 把加载失败渲染到卡片上（不再静默伪装成"无记录"，一眼看出链路断了）
function showCardError(listId, prefix) {
  const el = $(listId);
  if (el) el.innerHTML = '<p class="hint">' + prefix + "（详见 debug.log）</p>";
}

// 停用/提示类文案渲染到卡片
function showCardHint(listId, text) {
  const el = $(listId);
  if (el) el.innerHTML = '<p class="hint">' + text + "</p>";
}

// 成功结果只记前几次，防止 2 秒轮询刷爆日志
const audioLogged = { audio: 0, appu: 0 };

// 只画当前看得见的那部分：主界面画前 6 行，应用明细页画全量列表与柱状图
function paintAppUsage() {
  const s = viewData.appu;
  if (!s) return;
  const apps = s.apps || [];
  if (curView === "viewMain") {
    if (!isToday(s.date)) return; // 同上：主界面只反映今天
    const home = $("appuHomeList");
    if (home) renderAppRows(home, apps, 6);
    return;
  }
  if (curView !== "viewApp") return;
  updateDayNav("appu");
  const list = $("appuList");
  if (list) renderAppRows(list, apps, 0, dayEmpty(s.date, "应用使用记录"));
  renderHourChart($("appuChart"), s.hourly || [], isToday(s.date));
}

// 应用行：图标 + 软件名 + 进度条 + 时长（按时长降序）。emptyText 自定义空态文案
function renderAppRows(container, apps, limit, emptyText) {
  container.innerHTML = "";
  if (!apps.length) {
    container.innerHTML =
      '<p class="hint">' + (emptyText || "今日暂无应用使用记录") + "</p>";
    return;
  }
  const max = Math.max(1, apps[0].seconds);
  const shown = limit > 0 ? apps.slice(0, limit) : apps;
  for (const a of shown) {
    const row = document.createElement("div");
    row.className = "tk-row appu-row";
    row.innerHTML =
      appIconHTML(a) +
      '<span class="tk-key appu-name" title="' + escapeHtml(a.app) + '">' + escapeHtml(a.app) + "</span>" +
      '<div class="tk-bar"><div style="width:' + Math.round((a.seconds / max) * 100) + '%"></div></div>' +
      '<span class="tk-count appu-time">' + fmtDurCN(a.seconds) + "</span>";
    container.appendChild(row);
  }
  if (limit > 0 && apps.length > limit) {
    const more = document.createElement("p");
    more.className = "hint";
    more.textContent = "共 " + apps.length + " 个应用，点「查看使用明细」看全部";
    container.appendChild(more);
  }
}

// 图标：有 data URL 用 <img>，没有则显示首字占位
function appIconHTML(a) {
  if (a.icon) {
    return '<img class="app-icon" src="' + a.icon + '" alt="">';
  }
  const ch = (a.app || "?").trim().charAt(0) || "?";
  return '<span class="app-icon app-icon-fallback">' + escapeHtml(ch) + "</span>";
}

// 图标按应用名缓存（base64 data URL 几 KB~几十 KB，每 2 秒轮询整批回传纯属浪费）。
// 后端只回传前端缺的（known_icons 里没报过的），这里负责把新到的存起来、把缺的补回去。
// 应用使用与媒体播放共用一份：同一个应用两边图标相同，先加载的那页顺带喂给另一页。
const iconCache = new Map();

function mergeIcons(apps) {
  for (const a of apps || []) {
    if (a.icon) iconCache.set(a.app, a.icon);
    else if (iconCache.has(a.app)) a.icon = iconCache.get(a.app);
  }
}

function knownIcons() {
  return Array.from(iconCache.keys());
}

// 逐小时柱状图（复用 act-chart 样式；hourly 为 24 个秒数，空态文案自定义）
function renderHourChart(el, hourly, isToday = true) {
  if (!el) return;
  const now = isToday ? new Date().getHours() : -1;
  const max = Math.max(1, ...hourly);
  el.innerHTML = "";
  hourly.forEach((sec, i) => {
    const hgt = sec > 0 ? Math.max(5, Math.round((sec / max) * 100)) : 2;
    const d = document.createElement("div");
    d.className = "bar" + (sec > 0 ? "" : " empty") + (i === now ? " cur" : "");
    d.style.height = hgt + "%";
    d.title = i + "时 · " + (sec > 0 ? fmtDurCN(sec) : "无使用");
    el.appendChild(d);
  });
}

// ---- 媒体播放（audio_usage）：与「应用使用」完全独立的第二套统计 ----
async function loadAudioUsage() {
  if (!monitors.audio) {
    viewData.audio = null; // 同上，停用即清缓存
    if (curView === "viewMain") {
      showCardHint("audioHomeList", "已在设置中关闭媒体播放监控");
    }
    return;
  }
  try {
    const s = await invoke("get_audio_usage_summary", {
      knownIcons: knownIcons(),
      date: histDate.audio,
    });
    mergeIcons(s.apps || []); // 与应用使用共用一份图标缓存，同样与当前视图无关
    viewData.audio = s;
    paintAudioUsage();
    if (audioLogged.audio < 3) {
      audioLogged.audio++;
      const top =
        s.apps && s.apps[0] ? s.apps[0].app + "=" + s.apps[0].seconds + "s" : "empty";
      flog(
        "audio ok #" +
          audioLogged.audio +
          ": apps=" +
          (s.apps ? s.apps.length : "?") +
          " [" +
          top +
          "] watch_ok=" +
          s.watch_ok
      );
    }
  } catch (e) {
    flog("audio ERR: " + (e && e.message ? e.message : String(e)));
    showCardError("audioHomeList", "媒体播放数据加载失败");
    console.error("loadAudioUsage error:", e);
  }
}

// 只画当前看得见的那部分：主界面画前 6 行，媒体明细页画全量列表与柱状图
function paintAudioUsage() {
  const s = viewData.audio;
  if (!s) return;
  const apps = s.apps || [];
  if (curView === "viewMain") {
    if (!isToday(s.date)) return; // 同上：主界面只反映今天
    const home = $("audioHomeList");
    if (home) renderAppRows(home, apps, 6, "今日暂无播放记录");
    return;
  }
  if (curView !== "viewAudio") return;
  updateDayNav("audio");
  const list = $("audioList");
  if (list) renderAppRows(list, apps, 0, dayEmpty(s.date, "播放记录"));
  renderHourChart($("audioChart"), s.hourly || [], isToday(s.date));
}

// 文本/数字/时间控件：失去焦点时自动保存
[
  "monthly_salary",
  "am_start",
  "am_end",
  "pm_start",
  "pm_end",
  "payday",
  "workdays_override",
  "weekend_ot_start",
  "overtime_rate_weekend",
  "overtime_rate_holiday",
].forEach((id) => $(id).addEventListener("blur", saveIfChanged));
// 下拉框：选择即保存
$("duration_format").addEventListener("change", saveIfChanged);
// 副标题风格：立即保存，并用最近一次状态重画（不发 IPC）
$("tagline_style").addEventListener("change", () => {
  applyTaglineCustomVisibility($("tagline_style").value);
  saveNow();
  if (lastStatus) renderTagline(lastStatus);
  else tick();
});
// 自定义文案：输入时实时预览，失焦才写盘
$("tagline_custom").addEventListener("input", () => {
  if (taglineStyle() === "custom" && lastStatus) renderTagline(lastStatus);
});
$("tagline_custom").addEventListener("blur", saveIfChanged);
// 开关：立即保存
$("tray_hover_card").addEventListener("change", saveNow);
// 加班设置：输入框失焦保存，开关立即保存
["overtime_start", "overtime_rate", "overtime_meal"].forEach((id) =>
  $(id).addEventListener("blur", saveIfChanged),
);
$("overtime_enabled").addEventListener("change", () => {
  const on = $("overtime_enabled").checked;
  // 关闭时立即隐藏主界面加班卡片；开启时立即重新拉取并展示
  applyOvertimeVisibility(on);
  saveNow();
  if (on) loadOvertime();
});
$("overtime_meal_enabled").addEventListener("change", saveNow);
$("weekend_overtime").addEventListener("change", () => {
  applyRestOvertimeVisibility($("weekend_overtime").checked);
  saveNow();
});
// 应用使用白名单：开关立即保存；添加按钮/回车新增 chip 并保存
$("app_whitelist_enabled").addEventListener("change", saveNow);
$("appWhitelistAdd").addEventListener("click", addWhitelistItem);
$("appWhitelistInput").addEventListener("keydown", (e) => {
  if (e.key === "Enter") {
    e.preventDefault();
    addWhitelistItem();
  }
});
// 监控开关：立即保存（后端即时生效）+ 同步内存状态刷新首页卡片
["monitor_activity", "monitor_app_usage", "monitor_audio"].forEach((id) =>
  $(id).addEventListener("change", () => {
    syncMonitorState();
    saveNow();
    // 立即刷新一次对应卡片（显示数据或停用提示）
    if (id === "monitor_activity") loadActivity();
    else if (id === "monitor_app_usage") loadAppUsage();
    else loadAudioUsage();
  }),
);
// ---- 开机自启（独立读写注册表，不进 config）----
async function loadAutostart() {
  try {
    $("launch_on_boot").checked = await invoke("get_autostart");
  } catch (e) {
    flog("get_autostart ERR: " + (e && e.message ? e.message : String(e)));
  }
}

$("launch_on_boot").addEventListener("change", async () => {
  const want = $("launch_on_boot").checked;
  try {
    const msg = await invoke("set_autostart", { enabled: want });
    showToast(msg, "ok");
  } catch (e) {
    // 写注册表失败要把开关拨回真实状态，否则界面显示与实际不符
    await loadAutostart();
    const detail = e && e.message ? e.message : String(e);
    flog("set_autostart ERR: " + detail);
    showToast("设置失败：" + detail, "err");
  }
});

$("refreshBtn").addEventListener("click", refresh);

// ---- 数据存储：占用展示与立即整理 ----
function fmtBytes(n) {
  const v = Number(n) || 0;
  if (v < 1024) return v + " B";
  if (v < 1024 * 1024) return (v / 1024).toFixed(1) + " KB";
  return (v / 1024 / 1024).toFixed(2) + " MB";
}

// 占比文案：极小的项（<0.1%）不能显示成 0.0%，否则看起来像没占空间
function stgPct(bytes, total) {
  if (!(total > 0)) return "0%";
  const p = ((Number(bytes) || 0) / total) * 100;
  if (p <= 0) return "0%";
  if (p < 0.1) return "<0.1%";
  return p.toFixed(1) + "%";
}

// 上次读到的总占用，整理后据此算出释放了多少（WAL 归零是主要收益）
let stgLastTotal = null;

function renderStorageInfo(info) {
  const el = $("storageInfo");
  if (!el || !info) return;
  // 后端结构体是 snake_case 序列化（Tauri 不转驼峰，见 overtime 的 cross_midnight），
  // 这里写成 totalBytes 会静默拿到 undefined → 总计 0 B、每行占比全 0
  const total = Number(info.total_bytes) || 0;
  stgLastTotal = total;
  // 0 字节的项不展示：空库时不该列一堆 0 出来占版面
  const slices = (info.slices || []).filter((s) => s.bytes > 0);
  if (!slices.length) {
    el.innerHTML = '<span class="hint">暂无占用数据</span>';
    return;
  }
  let h =
    '<div class="stg-total"><span>共占用</span><b>' +
    fmtBytes(total) +
    "</b></div>";
  h += '<div class="stg-bar">';
  slices.forEach(function (s) {
    h +=
      '<i class="stg-seg k-' +
      s.key +
      '" style="width:' +
      (total > 0 ? (s.bytes / total) * 100 : 0) +
      '%"></i>';
  });
  h += "</div><ul class=\"stg-list\">";
  slices.forEach(function (s) {
    h +=
      '<li><i class="stg-dot k-' +
      s.key +
      '"></i><span class="stg-name">' +
      s.label +
      '</span><span class="stg-size">' +
      fmtBytes(s.bytes) +
      '</span><span class="stg-pct">' +
      stgPct(s.bytes, total) +
      "</span>" +
      (s.rows && s.unit
        ? '<span class="stg-sub">' + s.rows + " " + s.unit + "</span>"
        : "") +
      "</li>";
  });
  h += "</ul>";
  if (info.approx) h += '<p class="stg-note">表级占用按行数比例估算</p>';
  if (info.earliest_date) {
    h += '<p class="stg-note">最早数据 ' + info.earliest_date + "</p>";
  }
  el.innerHTML = h;
}

async function loadStorageInfo() {
  try {
    renderStorageInfo(await invoke("get_storage_info"));
  } catch (e) {
    flog("get_storage_info ERR: " + (e && e.message ? e.message : String(e)));
    const el = $("storageInfo");
    if (el) el.textContent = "占用信息读取失败";
  }
}

async function runCleanup() {
  const btn = $("cleanupBtn");
  if (btn) { btn.disabled = true; btn.textContent = "整理中…"; }
  try {
    const info = await invoke("run_maintenance");
    const after = Number(info && info.total_bytes) || 0;
    // 报「释放了多少」而不是「现在多少」：整理后 WAL 归零，说现有大小看着像什么都没做
    const freed =
      stgLastTotal !== null && stgLastTotal > after ? stgLastTotal - after : 0;
    renderStorageInfo(info);
    showToast(
      freed > 0
        ? "整理完成，释放 " + fmtBytes(freed) + "（当前共 " + fmtBytes(after) + "）"
        : "整理完成，已无可以释放的空间",
      "ok"
    );
  } catch (e) {
    const detail = e && e.message ? e.message : String(e);
    flog("run_maintenance ERR: " + detail);
    showToast("整理失败：" + detail, "err");
  } finally {
    if (btn) { btn.disabled = false; btn.textContent = "立即整理"; }
  }
}

$("retention_days").addEventListener("change", saveNow);
$("cleanupBtn").addEventListener("click", runCleanup);
// 加班记录增删改
$("otAddBtn").addEventListener("click", openOtForm);
$("otPrevMonth").addEventListener("click", () => shiftOtMonth(-1));
$("otNextMonth").addEventListener("click", () => shiftOtMonth(1));
$("otfSave").addEventListener("click", submitOtForm);
$("otfCancel").addEventListener("click", hideOtForm);
// 顶部导航：主页 / 明细 / 设置 三个 tab。「明细」是聚合 tab，对应 4 个二级页，
// 具体显示哪个由 lastDetailView 记忆（默认加班——最常用的明细），由分段条二次切换
const DETAIL_VIEWS = ["viewOt", "viewAct", "viewApp", "viewAudio"];
let lastDetailView = "viewOt";

// 视图切换统一入口：主界面 / 设置 / 4 个明细分段互斥（只留一个 .app 可见）
function showView(id) {
  const target = $(id);
  if (!target) return; // 目标视图不存在则不操作，避免误隐藏所有视图
  if (curView === "viewSettings" && id !== "viewSettings") {
    saveIfChanged(); // 自定义副标题等可能还没失焦
  }
  document.querySelectorAll(".app").forEach((v) => v.classList.add("hidden"));
  target.classList.remove("hidden");
  curView = id;
  // 导航同步（契约见 scripts 目录的导航回归测试）：
  // 工具栏分段高亮——4 个明细视图都映射到「明细」聚合段；
  // 二级分段条——仅在明细视图显示，并高亮当前分段；齿轮在设置视图点亮
  const isDetail = DETAIL_VIEWS.includes(id);
  if (isDetail) lastDetailView = id;
  const navKey = isDetail ? "detail" : id;
  document.querySelectorAll(".seg-nav").forEach((b) => {
    if (b.closest("#segbar")) b.classList.toggle("active", b.dataset.nav === id);
    else b.classList.toggle("active", b.dataset.nav === navKey);
  });
  $("gearBtn").classList.toggle("active", id === "viewSettings");
  $("segbar").classList.toggle("hidden", !isDetail);
  if (id === "viewMain") resetHistDates();
  // 懒渲染下目标视图可能从未画过（或还停留在上次的数据），立刻补一次，
  // 否则要等下一个轮询周期才出内容。
  repaintCurrentView();
}

// 用缓存立即补画当前视图；缓存还没到就现拉一次，避免切过去看到空白
function repaintCurrentView() {
  if (viewData.activity) paintActivity();
  else loadActivity();
  if (viewData.appu) paintAppUsage();
  else loadAppUsage();
  if (viewData.audio) paintAudioUsage();
  else loadAudioUsage();
  // 加班明细是 10 秒轮询，切进去立即拉一次，免得最多等 10 秒才出内容
  if (curView === "viewOt") loadOvertime();
}
$("otDetailBtn").addEventListener("click", () => showView("viewOt"));
// 加班战果行 → 加班明细（自动归入工具栏「明细」段）
// 今日监控卡（活动/应用/媒体三合一）：内联小分段切换预览面板，
// 「查看明细」跟随当前分段跳对应明细页。预览面板内的统计 id 由现有 paint 函数维护
let curMon = "act"; // 当前监控预览分段（act/app/audio）
const MON_PANES = { act: "monAct", app: "monApp", audio: "monAudio" };
const MON_VIEWS = { act: "viewAct", app: "viewApp", audio: "viewAudio" };
document.querySelectorAll(".mon-seg-item").forEach((btn) => {
  btn.addEventListener("click", () => {
    curMon = btn.dataset.mon;
    document.querySelectorAll(".mon-seg-item").forEach((b) => {
      b.classList.toggle("active", b === btn);
    });
    Object.entries(MON_PANES).forEach(([key, id]) =>
      $(id).classList.toggle("hidden", key !== curMon)
    );
  });
});
$("monDetailBtn").addEventListener("click", () => showView(MON_VIEWS[curMon]));
// 明细页的日期导航：翻天后立即按新日期重新拉取
$("actPrevDay").addEventListener("click", () => shiftHist("act", -1, loadActivity));
$("actNextDay").addEventListener("click", () => shiftHist("act", 1, loadActivity));
$("appuPrevDay").addEventListener("click", () => shiftHist("appu", -1, loadAppUsage));
$("appuNextDay").addEventListener("click", () => shiftHist("appu", 1, loadAppUsage));
$("audioPrevDay").addEventListener("click", () => shiftHist("audio", -1, loadAudioUsage));
$("audioNextDay").addEventListener("click", () => shiftHist("audio", 1, loadAudioUsage));

// 分段工具栏 + ⚙ 设置 + 明细二级分段条：任意视图直达（取代原「‹ 返回」的网页式导航）。
// 自动保存（离开设置页）与历史日期复位（回主页）仍由 showView 统一处理
document.querySelectorAll(".seg-nav").forEach((btn) => {
  btn.addEventListener("click", () =>
    showView(btn.dataset.nav === "detail" ? lastDetailView : btn.dataset.nav)
  );
});
$("gearBtn").addEventListener("click", () => showView("viewSettings"));

// 启动即把三页导航条初始化为「今天」，并禁用「后一天」
for (const k of Object.keys(DAY_NAV)) updateDayNav(k);

// 关闭窗口：若有未保存改动先落盘再隐藏，不丢数据
async function closeWindow() {
  if (JSON.stringify(readCfg()) !== lastSaved) {
    await doSave();
  }
  winVisible = false; // 立即停轮询，不等 Rust 端下一秒的可见性广播
  TAURI.window.getCurrentWindow().hide();
}
// Esc 键关闭（关闭交给原生窗口标题栏按钮；Esc 作为键盘快捷键保留）
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") closeWindow();
});

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// 启动页淡出：确保主界面数据就位后再收起，避免露出空值界面
function hideSplash() {
  const s = document.getElementById("splash");
  if (!s || s.classList.contains("hide")) return;
  s.classList.add("hide");
  setTimeout(() => { s.style.display = "none"; }, 500);
}

// 显示主窗口：窗口初始 visible:false（避免原生白闪）；splash 已在内存渲染好，
// 此刻 show 即深色画面，绝无白闪。Rust 端另有 1s 兜底 show 防 JS 异常。
async function showWindow() {
  try {
    await TAURI.window.getCurrentWindow().show();
  } catch (e) {
    console.error("showWindow", e);
  }
}

async function boot() {
  // 关键证据：记录 WebView2 实际加载的 URL（?v=a4197cb2 = 新前端；旧值 = 缓存没刷新）
  flog("boot: url=" + location.href + " ua=" + navigator.userAgent.slice(0, 60));
  // 尽早显示窗口（此刻 splash 已渲染成深色，show 无白闪）
  await showWindow();
  winVisible = true; // 窗口已显示，恢复轮询（启动时若先收到 false 也能被这里纠正）
  const shownAt = Date.now();
  try { await load(); } catch (e) { console.error("load", e); }
  try { await tick(); } catch (e) { console.error("tick", e); }
  try { await loadOvertime(); } catch (e) { console.error("ot", e); }
  try { await loadActivity(); } catch (e) { console.error("act", e); }
  try { await loadAppUsage(); } catch (e) { console.error("appu", e); }
  try { await loadAudioUsage(); } catch (e) { console.error("audio", e); }
  // 节假日刷新在后台进行，不阻塞启动页
  refresh();
  // 启动页至少显示 2 秒
  const elapsed = Date.now() - shownAt;
  if (elapsed < 2000) await sleep(2000 - elapsed);
  hideSplash();
}

// 窗口重新可见时补跑一轮：隐藏期间数据照常在后台累积，切回来要立刻看到最新值
function refreshAll() {
  tick();
  loadOvertime();
  loadActivity();
  loadAppUsage();
  loadAudioUsage();
}

// 订阅 Rust 端广播的主窗口可见性变化。事件系统若不通则 winVisible 保持 true，
// 退化成「始终轮询」的旧行为——宁可浪费，也不能让界面不刷新。
async function watchVisibility() {
  try {
    await TAURI.event.listen("win-visibility", (e) => {
      const vis = !!(e && e.payload);
      if (vis === winVisible) return;
      winVisible = vis;
      if (vis) refreshAll();
    });
  } catch (err) {
    flog("vis listen failed: " + (err && err.message ? err.message : String(err)));
  }
}

boot();
watchVisibility();
// 以下轮询全部受 winVisible 约束：窗口 hide 到托盘时直接跳过，
// 不再空跑「每 2 秒三次 IPC + SQLite 聚合查询 + 图标 base64 回传」。
setInterval(() => { if (winVisible) tick(); }, 1000);
// 加班记录每 10 秒刷新（锁屏=下班离开事件可能随时产生新记录）
setInterval(() => { if (winVisible) loadOvertime(); }, 10000);
// 活动统计每 2 秒刷新（命令内部会先刷内存计数，点击/按键后近实时可见）
setInterval(() => { if (winVisible) loadActivity(); }, 2000);
// 应用使用每 2 秒刷新（10 秒结算一次，2 秒轮询保证切回后尽快看到新值）
// 媒体播放每 2 秒刷新（5 秒结算一次，口径同上）
// 三者错开 700ms 相位：原先同时刻三连发会堆出一个卡顿尖峰，错开后每秒最多一次 IPC
setTimeout(() => setInterval(() => { if (winVisible) loadAppUsage(); }, 2000), 700);
setTimeout(() => setInterval(() => { if (winVisible) loadAudioUsage(); }, 2000), 1400);
