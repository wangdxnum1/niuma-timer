
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
    // 旧配置缺该字段时后端 serde 默认下发 true，这里 !== false 与之对齐
    $("overtime_exclude_remote").checked = cfg.overtime_exclude_remote !== false;
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
    // 守护设置（v1.3.0）：开关缺省 true 与后端 serde default 对齐，阈值缺省 50
    $("remind_sedentary_enabled").checked = cfg.remind_sedentary_enabled !== false;
    $("remind_sedentary_minutes").value = cfg.remind_sedentary_minutes ?? 50;
    $("remind_offwork_enabled").checked = cfg.remind_offwork_enabled !== false;
    $("remind_payday_enabled").checked = cfg.remind_payday_enabled !== false;
    $("shortcuts_enabled").checked = cfg.shortcuts_enabled !== false;
    // 自动更新（v1.4.0）：缺省 true，与后端 serde default 对齐
    $("update_auto_check").checked = cfg.update_auto_check !== false;
    $("retention_days").value = String(cfg.retention_days || 0);
    setBillStyleUI(cfg.bill_style || "receipt");
    setBillSpanUI(cfg.bill_span || "week");
    loadStorageInfo();
    loadBackups();
    // 升级后公告（v1.4.0）：后端只在下发一次后清空，非空即直接进更新页
    try {
      const ann = await invoke("take_update_announcement");
      if (ann) {
        updateAnnounce = ann;
        showView("viewUpdate");
      }
    } catch (e) {
      flog("take_update_announcement ERR: " + (e && e.message ? e.message : String(e)));
    }
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
    overtime_exclude_remote: $("overtime_exclude_remote").checked,
    overtime_start: $("overtime_start").value || null,
    overtime_rate: parseFloat($("overtime_rate").value) || 0,
    overtime_meal_enabled: $("overtime_meal_enabled").checked,
    overtime_meal: parseFloat($("overtime_meal").value) || 0,
    monitor_activity: $("monitor_activity").checked,
    monitor_app_usage: $("monitor_app_usage").checked,
    monitor_audio: $("monitor_audio").checked,
    // 守护（v1.3.0）：阈值钳制在设计区间 1–120，空值回退默认 50
    remind_sedentary_enabled: $("remind_sedentary_enabled").checked,
    remind_sedentary_minutes: Math.min(
      120,
      Math.max(1, parseInt($("remind_sedentary_minutes").value, 10) || 50)
    ),
    remind_offwork_enabled: $("remind_offwork_enabled").checked,
    remind_payday_enabled: $("remind_payday_enabled").checked,
    shortcuts_enabled: $("shortcuts_enabled").checked,
    // 自动更新（v1.4.0）：必须带回去，否则 merge 时一直保留旧值
    update_auto_check: $("update_auto_check").checked,
    weekend_overtime: $("weekend_overtime").checked,
    weekend_ot_start: $("weekend_ot_start").value || null,
    overtime_rate_weekend: numOrNull($("overtime_rate_weekend").value),
    overtime_rate_holiday: numOrNull($("overtime_rate_holiday").value),
    app_whitelist_enabled: $("app_whitelist_enabled").checked,
    app_whitelist: readWhitelist(),
    retention_days: parseInt($("retention_days").value) || 0,
    bill_style: readBillStyle(),
    bill_span: readBillSpan(),
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
function saveIfChanged(options = {}) {
  if (JSON.stringify(readCfg()) === lastSaved) return;
  return doSave(options);
}

// 开关等明确变更：直接保存
function saveNow() {
  doSave();
}

async function doSave({ silent = false } = {}) {
  const cfg = readCfg();
  try {
    await invoke("save_config", { cfg });
    lastSaved = JSON.stringify(cfg);
    if (!silent) showToast("已自动保存", "ok");
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

