
async function load() {
  try {
    const cfg = await invoke("load_config");
    $("monthly_salary").value = cfg.monthly_salary;
    setSalaryModeUI(cfg.salary_mode || "monthly");
    // 0/缺省显示为空串：空值保存时不传字段，不会把用户已配的时薪抹成 0
    $("hourly_wage").value = cfg.hourly_wage ? cfg.hourly_wage : "";
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
    // 专注段（v1.7.0）：开关缺省 true，阈值缺省 25
    $("focus_enabled").checked = cfg.focus_enabled !== false;
    $("focus_min_minutes").value = cfg.focus_min_minutes ?? 25;
    applyFocusVisibility(cfg.focus_enabled !== false);
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
    // 加载成功才解锁自动保存（见 doSave 门闸）；同时撤掉失败横幅
    configLoaded = true;
    showCfgLoadError(false);
  } catch (e) {
    flog("load_config ERR: " + (e && e.message ? e.message : String(e)));
    console.error(e);
    // 加载失败 = 表单还是空白默认值。此时放行自动保存，用户随手拨一个开关
    // 就会把空作息/空薪资 merge 进真实配置（merge 按键无条件覆盖）——
    // 宁可拒绝保存，横幅里给「重新加载」按钮。
    configLoaded = false;
    showCfgLoadError(true);
  }
}

// 设置加载失败横幅：显隐由 load() 控制，重试按钮重新走一遍 load()
function showCfgLoadError(show) {
  const el = $("cfgLoadError");
  if (el) el.classList.toggle("hidden", !show);
}

// ---- 自动保存（控件失去焦点时触发）----
let lastSaved = null; // 上次成功保存的配置 JSON 快照，用于去重
let lastOverride = null; // 上次保存的上班天数，用于判断是否需静默刷新工作日数据
// 配置是否加载成功。false 时自动保存全部拒绝——这是「用空表单覆盖真实配置」的门闸
let configLoaded = false;

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
  const wageRaw = $("hourly_wage").value.trim();
  const paydayRaw = $("payday").value.trim();
  const overrideRaw = $("workdays_override").value.trim();
  const cfg = {
    salary_mode: readSalaryMode(),
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
    focus_enabled: $("focus_enabled").checked,
    focus_min_minutes: Math.min(
      120,
      Math.max(10, parseInt($("focus_min_minutes").value, 10) || 25)
    ),
    shortcuts_enabled: $("shortcuts_enabled").checked,
    // 自动更新（v1.4.0）：必须带回去，否则 merge 时一直保留旧值
    update_auto_check: $("update_auto_check").checked,
    weekend_overtime: $("weekend_overtime").checked,
    weekend_ot_start: $("weekend_ot_start").value || null,
    overtime_rate_weekend: numOrNull($("overtime_rate_weekend").value),
    overtime_rate_holiday: numOrNull($("overtime_rate_holiday").value),
    app_whitelist_enabled: $("app_whitelist_enabled").checked,
    app_whitelist: readWhitelist(),
    // 乱输负数/超长都会静默变成奇怪语义，钳到 0（永久）–36500（约百年）
    retention_days: Math.min(36500, Math.max(0, parseInt($("retention_days").value) || 0)),
    bill_style: readBillStyle(),
    bill_span: readBillSpan(),
  };
  // 月薪/发薪日留空：不传该字段，后端合并时保留旧值，避免误存 0/1，也不挡住其它开关保存
  if (salaryRaw !== "") cfg.monthly_salary = parseFloat(salaryRaw) || 0;
  if (wageRaw !== "") cfg.hourly_wage = parseFloat(wageRaw) || 0;
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

// ---- 计薪方式（v1.6.0）：monthly 月聘 / hourly 时薪 ----
function readSalaryMode() {
  const active = document.querySelector("#salaryModeSeg .mon-seg-item.active");
  return active ? active.dataset.salaryMode : "monthly";
}

function setSalaryModeUI(mode) {
  const m = mode === "hourly" ? "hourly" : "monthly";
  document.querySelectorAll("#salaryModeSeg .mon-seg-item").forEach((b) => {
    b.classList.toggle("active", b.dataset.salaryMode === m);
  });
  applySalaryModeVisibility();
}

// 时薪模式下月聘三件套（月薪/工作日覆盖/刷新按钮）无意义，整组隐藏；
// 控件值不清空——切回月聘时原配置原样回来
function applySalaryModeVisibility() {
  const hourly = readSalaryMode() === "hourly";
  $("hourlyWageRow").classList.toggle("hidden", !hourly);
  $("monthlySalaryRow").classList.toggle("hidden", hourly);
  $("workdaysOverrideRow").classList.toggle("hidden", hourly);
  $("refreshBtn").classList.toggle("hidden", hourly);
  $("workdaysInfo").classList.toggle("hidden", hourly);
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
  // 门闸：配置没加载成功时，表单里是空白默认值，此刻保存=用空值覆盖真实配置。
  // 静默保存（切跨度记偏好等）连 toast 都不打，只留日志。
  if (!configLoaded) {
    flog("doSave skipped: config not loaded (load_config failed at boot)");
    if (!silent) showToast("配置未加载，已阻止自动保存——请点设置页顶部「重新加载」", "err");
    return;
  }
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


// 加载失败横幅的「重新加载」按钮：属于本文件的加载职责，绑在这里而非 monitor.js。
// 脚本在 body 末尾加载，DOM 此时已就绪，可直接绑定。
$("cfgRetryBtn").addEventListener("click", async () => { await load(); });

// ---- 设置页控件绑定（自 monitor.js 迁回，2026-10-04 批次五：设置域逻辑与绑定同文件）----

// 文本/数字/时间控件：失去焦点时自动保存
[
  "monthly_salary",
  "hourly_wage",
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
// 计薪方式分段（v1.6.0）：切换即保存并显隐月聘/时薪字段
document.querySelectorAll("#salaryModeSeg .mon-seg-item").forEach((b) => {
  b.addEventListener("click", () => {
    setSalaryModeUI(b.dataset.salaryMode);
    saveNow();
  });
});
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
// 守护设置（v1.3.0）：开关即存；阈值失焦存
$("remind_sedentary_enabled").addEventListener("change", saveNow);
$("remind_offwork_enabled").addEventListener("change", saveNow);
$("remind_payday_enabled").addEventListener("change", saveNow);
// 专注段（v1.7.0）：开关即存，阈值失焦存
$("focus_enabled").addEventListener("change", () => {
  applyFocusVisibility($("focus_enabled").checked);
  saveNow();
});

// 专注统计关闭时隐藏达标线输入（同 overtime.js applyOvertimeVisibility 模式）：
// 此前开关只存盘不联动，焦点行永远显示——像坏了的开关
function applyFocusVisibility(enabled) {
  const row = $("focusMinutesRow");
  if (row) row.classList.toggle("hidden", !enabled);
}
$("focus_min_minutes").addEventListener("blur", saveIfChanged);
$("shortcuts_enabled").addEventListener("change", saveNow);
$("update_auto_check").addEventListener("change", saveNow);
$("remind_sedentary_minutes").addEventListener("blur", saveIfChanged);
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
$("overtime_exclude_remote").addEventListener("change", saveNow);
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
