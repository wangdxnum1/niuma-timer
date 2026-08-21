// 牛马计时器 主界面前端逻辑
// window.__TAURI__ 由 Rust 端 append_invoke_initialization_script 注入的垫片暴露
const TAURI = window.__TAURI__;
const invoke = TAURI.core.invoke;

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
    $("overtime_start").value = cfg.overtime_start || "";
    $("overtime_rate").value = cfg.overtime_rate ?? 20;
    $("overtime_meal_enabled").checked = !!cfg.overtime_meal_enabled;
    $("overtime_meal").value = cfg.overtime_meal ?? 20;
    // 初始快照：与 readCfg() 字段顺序一致，用于失焦保存时判断是否有变化
    lastSaved = JSON.stringify(readCfg());
  } catch (e) {
    console.error(e);
  }
}

// ---- 自动保存（控件失去焦点时触发）----
let lastSaved = null; // 上次成功保存的配置 JSON 快照，用于去重
let lastOverride = null; // 上次保存的上班天数，用于判断是否需静默刷新工作日数据

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
    weekend_overtime: false,
    last_holiday_year: 0,
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

// 加班记录加载与渲染
async function loadOvertime() {
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
      '<tr><td colspan="7" class="ot-empty">暂无加班记录</td></tr>';
    return;
  }
  for (const r of [...ot.records].reverse()) {
    const tr = document.createElement("tr");
    const cells = [
      r.date.slice(5),
      r.lock_time,
      r.valid_hours.toFixed(1) + "h",
      "¥" + r.fee.toFixed(0),
      r.meal > 0 ? "¥" + r.meal.toFixed(0) : "—",
      "¥" + r.total.toFixed(0),
    ];
    for (const c of cells) {
      const td = document.createElement("td");
      td.textContent = c;
      tr.appendChild(td);
    }
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
  saveNow();
  loadOvertime();
});
$("overtime_meal_enabled").addEventListener("change", saveNow);
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

// 关闭窗口：若有未保存改动先落盘再隐藏，不丢数据
async function closeWindow() {
  if (JSON.stringify(readCfg()) !== lastSaved) {
    await doSave();
  }
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
  // 尽早显示窗口（此刻 splash 已渲染成深色，show 无白闪）
  await showWindow();
  const shownAt = Date.now();
  try { await load(); } catch (e) { console.error("load", e); }
  try { await tick(); } catch (e) { console.error("tick", e); }
  try { await loadOvertime(); } catch (e) { console.error("ot", e); }
  // 节假日刷新在后台进行，不阻塞启动页
  refresh();
  // 启动页至少显示 2 秒
  const elapsed = Date.now() - shownAt;
  if (elapsed < 2000) await sleep(2000 - elapsed);
  hideSplash();
}

boot();
setInterval(tick, 1000);
// 加班记录每 10 秒刷新（锁屏=下班离开事件可能随时产生新记录）
setInterval(loadOvertime, 10000);
