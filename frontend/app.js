// 牛马计时器 主界面前端逻辑
// window.__TAURI__ 由 Rust 端 append_invoke_initialization_script 注入的垫片暴露
const TAURI = window.__TAURI__;
const invoke = TAURI.core.invoke;

// 前端版本标记：写进每条日志，用于核对 WebView2 实际加载的是哪个版本（防旧缓存）
const FE_VER = "2026-09-07.v15";

// 主窗口是否可见。托盘常驻期间窗口是 hide 的，此时前端一切轮询都没意义
// （界面看不见，数据看不见），由 Rust 端 1s 线程广播 win-visibility 驱动。
// 初值 true：万一事件系统不通，退化为「始终轮询」的旧行为，不会更差。
let winVisible = true;

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
    $("workdays_override").value = cfg.workdays_override ?? "";
    lastOverride = cfg.workdays_override ?? null;
    $("overtime_enabled").checked = !!cfg.overtime_enabled;
    applyOvertimeVisibility(cfg.overtime_enabled);
    $("overtime_start").value = cfg.overtime_start || "";
    $("overtime_rate").value = cfg.overtime_rate ?? 20;
    $("overtime_meal_enabled").checked = !!cfg.overtime_meal_enabled;
    $("overtime_meal").value = cfg.overtime_meal ?? 20;
    $("weekend_overtime").checked = !!cfg.weekend_overtime;
    $("app_whitelist_enabled").checked = !!cfg.app_whitelist_enabled;
    renderWhitelist(cfg.app_whitelist || []);
    $("monitor_activity").checked = cfg.monitor_activity !== false;
    $("monitor_app_usage").checked = cfg.monitor_app_usage !== false;
    $("monitor_audio").checked = cfg.monitor_audio !== false;
    syncMonitorState();
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

function readCfg() {
  return {
    monthly_salary: parseFloat($("monthly_salary").value) || 0,
    am_start: $("am_start").value,
    am_end: $("am_end").value,
    pm_start: $("pm_start").value,
    pm_end: $("pm_end").value,
    payday: parseInt($("payday").value) || 1,
    duration_format: $("duration_format").value || "hms",
    tray_hover_card: $("tray_hover_card").checked,
    workdays_override: $("workdays_override").value
      ? parseInt($("workdays_override").value)
      : null,
    overtime_enabled: $("overtime_enabled").checked,
    overtime_start: $("overtime_start").value || null,
    overtime_rate: parseFloat($("overtime_rate").value) || 0,
    overtime_meal_enabled: $("overtime_meal_enabled").checked,
    overtime_meal: parseFloat($("overtime_meal").value) || 0,
    monitor_activity: $("monitor_activity").checked,
    monitor_app_usage: $("monitor_app_usage").checked,
    monitor_audio: $("monitor_audio").checked,
    weekend_overtime: $("weekend_overtime").checked,
    app_whitelist_enabled: $("app_whitelist_enabled").checked,
    app_whitelist: readWhitelist(),
  };
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
  // 月薪/发薪日为空时（用户清空了输入）不保存，保留旧值，避免误存 0/1
  if ($("monthly_salary").value.trim() === "" || $("payday").value.trim() === "") {
    return;
  }
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

async function tick() {
  try {
    const s = await invoke("get_status_cmd");
    $("earned").textContent = "¥" + s.earned.toFixed(2);
    $("worked").textContent = s.worked_str || s.worked_h.toFixed(1) + "h";
    $("toff").textContent = s.to_off_str || (s.off_work ? "已下班" : "—");
    $("rate").textContent = "¥" + s.rate_per_min.toFixed(2) + "/分";
    $("pay").textContent = s.days_to_pay + " 天";
  } catch (e) {
    /* 忽略瞬时错误 */
  }
}

// 加班开关关闭时隐藏主界面「加班总览」卡片；已有数据保留在库中不受影响
function applyOvertimeVisibility(enabled) {
  const card = $("otCard");
  if (card) card.style.display = enabled ? "" : "none";
}

// 加班记录加载与渲染
async function loadOvertime() {
  // 加班追踪关闭时不拉取、不展示（历史记录仍保留在 SQLite，开关不影响数据）
  if (!$("overtime_enabled").checked) return;
  try {
    const ot = await invoke("get_overtime_records");
    renderOt(ot);
  } catch (e) {
    console.error("loadOvertime error:", e);
  }
}

function renderOt(ot) {
  const now = new Date();
  $("otMonthTitle").textContent =
    now.getFullYear() + "年" + (now.getMonth() + 1) + "月 加班明细";
  $("ot_total").textContent = "¥" + ot.total_all.toFixed(0);
  $("ot_hours").textContent = ot.total_hours.toFixed(1) + "h";
  $("ot_days").textContent = ot.days;
  $("ot_meal_total").textContent = "¥" + ot.total_meal.toFixed(0);
  $("ot_avg").textContent =
    ot.days > 0 ? "¥" + (ot.total_all / ot.days).toFixed(0) : "¥0";
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
      r.lock_time,
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
  if (!date || !lock) {
    showOtMsg("请填写日期和下班时间");
    return;
  }
  // 前端先把关：只能当月
  const parts = date.split("-");
  const now = new Date();
  if (+parts[0] !== now.getFullYear() || +parts[1] - 1 !== now.getMonth()) {
    showOtMsg("只能添加/修改当月的数据");
    return;
  }
  try {
    const view = await invoke("save_overtime_record", {
      input: { date, lock_time: lock, ot_start: start },
    });
    renderOt(view);
    hideOtForm();
    showToast("已保存加班记录", "ok");
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
    renderOt(view);
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

// 某小时桶的简要描述（tooltip 用）
function fmtBucket(b, i) {
  const parts = [];
  if (b.moves) parts.push("移动 " + b.moves.toLocaleString());
  if (b.left) parts.push("点击 " + b.left.toLocaleString());
  if (b.keys) parts.push("按键 " + b.keys.toLocaleString());
  if (b.wheel) parts.push("滚轮 " + b.wheel + " 次");
  return i + "时 · " + (parts.length ? parts.join("、") : "无活动");
}

async function loadActivity() {
  if (!monitors.activity) {
    // 已停用：首页卡片显示占位符
    ["act_left", "act_right", "act_keys", "act_hours"].forEach((id) => ($(id).textContent = "—"));
    return;
  }
  try {
    const a = await invoke("get_activity_summary");
    renderActivity(a);
  } catch (e) {
    console.error("loadActivity error:", e);
  }
}

function renderActivity(a) {
  const t = a.totals || {};
  // 主界面汇总卡片
  $("act_left").textContent = (t.left || 0).toLocaleString();
  $("act_right").textContent = (t.right || 0).toLocaleString();
  $("act_keys").textContent = (t.keys || 0).toLocaleString();
  $("act_hours").textContent = (a.active_hours || 0) + "h";
  // 二级页明细
  $("actL").textContent = (t.left || 0).toLocaleString();
  $("actD").textContent = (t.dbl || 0).toLocaleString();
  $("actR").textContent = (t.right || 0).toLocaleString();
  $("actW").textContent = (t.wheel || 0) + " 次 · " + (t.wheel_ticks || 0) + " 格";
  $("actM").textContent = (t.moves || 0).toLocaleString();
  $("actK").textContent = (t.keys || 0).toLocaleString();
  $("actP").textContent = fmtDist(t.pixels);
  $("actH").textContent = (a.active_hours || 0) + " 小时";
  renderChart(a.hourly || []);
  renderTopKeys(a.top_keys || []);
}

// 逐小时活跃柱状图（纯 CSS，无第三方库）
function renderChart(hourly) {
  const el = $("actChart");
  if (!el) return;
  const now = new Date().getHours();
  const max = Math.max(
    1,
    ...hourly.map((b) => (b.moves || 0) + (b.left || 0) + (b.keys || 0))
  );
  el.innerHTML = "";
  hourly.forEach((b, i) => {
    const v =
      (b.moves || 0) + (b.left || 0) + (b.dbl || 0) + (b.right || 0) +
      (b.wheel || 0) + (b.mid || 0) + (b.xbtn || 0) + (b.keys || 0);
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
    showCardHint("appuHomeList", "已在设置中关闭应用使用监控");
    return;
  }
  try {
    const s = await invoke("get_app_usage_summary", { known_icons: knownIcons() });
    renderAppUsage(s);
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

function renderAppUsage(s) {
  const apps = s.apps || [];
  mergeIcons(apps);
  // 首页卡片：排行前 6
  const home = $("appuHomeList");
  if (home) renderAppRows(home, apps, 6);
  // 明细页：全部
  const list = $("appuList");
  if (list) renderAppRows(list, apps, 0);
  renderHourChart($("appuChart"), s.hourly || []);
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
function renderHourChart(el, hourly) {
  if (!el) return;
  const now = new Date().getHours();
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
    showCardHint("audioHomeList", "已在设置中关闭媒体播放监控");
    return;
  }
  try {
    const s = await invoke("get_audio_usage_summary", { known_icons: knownIcons() });
    renderAudioUsage(s);
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

function renderAudioUsage(s) {
  const apps = s.apps || [];
  mergeIcons(apps);
  // 首页卡片：排行前 6
  const home = $("audioHomeList");
  if (home) renderAppRows(home, apps, 6, "今日暂无播放记录");
  // 明细页：全部
  const list = $("audioList");
  if (list) renderAppRows(list, apps, 0, "今日暂无播放记录");
  renderHourChart($("audioChart"), s.hourly || []);
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
].forEach((id) => $(id).addEventListener("blur", saveIfChanged));
// 下拉框：选择即保存
$("duration_format").addEventListener("change", saveIfChanged);
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
$("weekend_overtime").addEventListener("change", saveNow);
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
$("refreshBtn").addEventListener("click", refresh);
// 加班记录增删改
$("otAddBtn").addEventListener("click", openOtForm);
$("otfSave").addEventListener("click", submitOtForm);
$("otfCancel").addEventListener("click", hideOtForm);
// 主界面 ↔ 加班明细二级页面切换
function showView(id) {
  const target = $(id);
  if (!target) return; // 目标视图不存在则不操作，避免误隐藏所有视图
  document.querySelectorAll(".app").forEach((v) => v.classList.add("hidden"));
  target.classList.remove("hidden");
}
$("otDetailBtn").addEventListener("click", () => showView("viewOt"));
$("otBackBtn").addEventListener("click", () => showView("viewMain"));
// 主界面 ↔ 活动明细二级页面切换
$("actDetailBtn").addEventListener("click", () => showView("viewAct"));
$("actBackBtn").addEventListener("click", () => showView("viewMain"));
// 主界面 ↔ 应用使用明细二级页面切换
$("appuDetailBtn").addEventListener("click", () => showView("viewApp"));
$("appuBackBtn").addEventListener("click", () => showView("viewMain"));
// 主界面 ↔ 媒体播放明细二级页面切换
$("audioDetailBtn").addEventListener("click", () => showView("viewAudio"));
$("audioBackBtn").addEventListener("click", () => showView("viewMain"));
// 主界面 ↔ 设置二级页面切换
$("settingsBtn").addEventListener("click", () => showView("viewSettings"));
$("settingsBackBtn").addEventListener("click", () => showView("viewMain"));

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
  // 关键证据：记录 WebView2 实际加载的 URL（?v=8 = 新前端；旧值 = 缓存没刷新）
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
setInterval(() => { if (winVisible) loadAppUsage(); }, 2000);
// 媒体播放每 2 秒刷新（5 秒结算一次，口径同上）
setInterval(() => { if (winVisible) loadAudioUsage(); }, 2000);
