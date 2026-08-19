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
  } catch (e) {
    console.error(e);
  }
}

async function save() {
  const cfg = {
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
    last_holiday_year: 0,
  };
  try {
    await invoke("save_config", { cfg });
    await refresh();
    $("workdaysInfo").textContent = "已保存 ✓";
  } catch (e) {
    $("workdaysInfo").textContent = "保存失败：" + e;
  }
}

async function refresh() {
  try {
    const n = await invoke("refresh_holidays");
    $("workdaysInfo").textContent = "当月实际上班天数：" + n + " 天";
  } catch (e) {
    $("workdaysInfo").textContent = "刷新失败：" + e;
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

$("saveBtn").addEventListener("click", save);
$("refreshBtn").addEventListener("click", refresh);
$("closeBtn").addEventListener("click", () =>
  TAURI.window.getCurrentWindow().hide(),
);

load();
refresh();
tick();
setInterval(tick, 1000);
