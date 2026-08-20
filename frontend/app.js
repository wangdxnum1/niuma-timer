// 牛马计时器 设置页前端逻辑
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
        '<tr><td colspan="6" class="ot-empty">暂无加班记录</td></tr>';
      return;
    }
    for (const r of [...ot.records].reverse()) {
      const tr = document.createElement("tr");
      tr.innerHTML =
        "<td>" + r.date.slice(5) + "</td>" +
        "<td>" + r.lock_time + "</td>" +
        "<td>" + r.valid_hours.toFixed(1) + "h</td>" +
        "<td>¥" + r.fee.toFixed(0) + "</td>" +
        "<td>" + (r.meal > 0 ? "¥" + r.meal.toFixed(0) : "—") + "</td>" +
        "<td>¥" + r.total.toFixed(0) + "</td>";
      tbody.appendChild(tr);
    }
  } catch (e) {
    console.error("loadOvertime error:", e);
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

// 关闭窗口：若有未保存改动先落盘再隐藏，不丢数据
async function closeWindow() {
  if (JSON.stringify(readCfg()) !== lastSaved) {
    await doSave();
  }
  TAURI.window.getCurrentWindow().hide();
}
// Esc 键关闭
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") closeWindow();
});
// 点击窗口外部（失焦）自动关闭
window.addEventListener("blur", closeWindow);

load();
refresh();
tick();
loadOvertime();
setInterval(tick, 1000);
// 加班记录每 10 秒刷新（锁屏事件可能随时产生新记录）
setInterval(loadOvertime, 10000);
