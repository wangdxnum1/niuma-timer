// Settings presentation and validation. Persistence remains in settings.js.
function settingsOverrideIsCurrent(cfg, yearMonth) {
  return cfg.workdays_override != null && cfg.workdays_override !== "" &&
    cfg.workdays_override_for === yearMonth;
}

function settingsScheduleModel(cfg, monthlyWorkdays) {
  const times = [cfg.am_start, cfg.am_end, cfg.pm_start, cfg.pm_end].map(value =>
    /^(?:[01]\d|2[0-3]):[0-5]\d$/.test(value || "") ? minutesOf(value) : null);
  const [amStart, amEnd, pmStart, pmEnd] = times;
  const valid = times.every(t => t !== null && t >= 0 && t < 1440) &&
    amEnd >= amStart && pmEnd >= pmStart && pmStart >= amEnd &&
    (amEnd - amStart + pmEnd - pmStart) > 0;
  if (!valid) return { valid: false, hours: null, hourlyRate: null, dailyPay: null,
    message: "请检查作息：结束不能早于开始，两段不能重叠，全天工时需大于 0" };
  const hours = (amEnd - amStart + pmEnd - pmStart) / 60;
  const raw = cfg.salary_mode === "hourly" ? cfg.hourly_wage : cfg.monthly_salary;
  const amount = raw === "" || raw == null ? null : Number(raw);
  const days = Number(monthlyWorkdays);
  const hourlyRate = amount === null || !Number.isFinite(amount) || amount < 0 ? null :
    cfg.salary_mode === "hourly" ? amount : days > 0 ? amount / days / hours : null;
  return { valid: true, hours, hourlyRate, dailyPay: hourlyRate === null ? null : hourlyRate * hours,
    message: hourlyRate === null ? "完善薪资或获取工作日数据后显示计薪预览" : "" };
}

function settingsRateModel(cfg) {
  const absent = value => value === "" || value == null;
  const weekend = absent(cfg.overtime_rate_weekend) ? Number(cfg.overtime_rate) : Number(cfg.overtime_rate_weekend);
  const holiday = absent(cfg.overtime_rate_holiday) ? weekend : Number(cfg.overtime_rate_holiday);
  return { weekend, holiday };
}

function settingsValidationErrors(cfg) {
  const errors = [];
  const number = (id, min, max = Infinity, optional = false, integer = false) => {
    const raw = cfg[id];
    if (optional && (raw === "" || raw == null)) return;
    const n = Number(raw);
    if (raw === "" || raw == null || !Number.isFinite(n) || n < min || n > max || (integer && !Number.isInteger(n))) {
      errors.push({ id, message: max === Infinity ? "请输入不小于 " + min + " 的金额" :
        "请输入 " + min + "–" + max + (integer ? " 之间的整数" : " 之间的数值") });
    }
  };
  number(cfg.salary_mode === "hourly" ? "hourly_wage" : "monthly_salary", 0, Infinity, true);
  number("payday", 1, 31, true, true);
  number("workdays_override", 1, 31, !cfg.workdays_manual || cfg.salary_mode === "hourly", true);
  if (!settingsScheduleModel(cfg, null).valid) {
    errors.push({ id: "am_start", message: "请检查时间顺序，两段不能重叠，全天工时需大于 0" });
  }
  if (cfg.overtime_enabled) {
    number("overtime_rate", 0);
    if (cfg.overtime_meal_enabled) number("overtime_meal", 0);
    if (cfg.weekend_overtime) {
      number("overtime_rate_weekend", 0, Infinity, true);
      number("overtime_rate_holiday", 0, Infinity, true);
    }
  }
  if (cfg.remind_sedentary_enabled) number("remind_sedentary_minutes", 1, 120, false, true);
  if (cfg.focus_enabled) number("focus_min_minutes", 10, 120, false, true);
  return errors;
}

function settingsFieldIsActive(id, cfg) {
  if (id === "monthly_salary") return cfg.salary_mode !== "hourly";
  if (id === "hourly_wage") return cfg.salary_mode === "hourly";
  if (id === "workdays_override") return cfg.salary_mode !== "hourly" && cfg.workdays_manual;
  if (id === "remind_sedentary_minutes") return cfg.remind_sedentary_enabled;
  if (id === "focus_min_minutes") return cfg.focus_enabled;
  if (["overtime_start", "overtime_rate"].includes(id)) return cfg.overtime_enabled;
  if (id === "overtime_meal") return cfg.overtime_enabled && cfg.overtime_meal_enabled;
  if (["weekend_ot_start", "overtime_rate_weekend", "overtime_rate_holiday"].includes(id)) return cfg.overtime_enabled && cfg.weekend_overtime;
  return true;
}

function settingsPreserveInactiveValues(cfg, saved, raw = cfg) {
  const active = {...cfg, workdays_manual: settingsManualDays};
  const optional = ["monthly_salary", "hourly_wage", "overtime_rate_weekend", "overtime_rate_holiday"];
  for (const id of ["monthly_salary", "hourly_wage", "overtime_start", "overtime_rate", "overtime_meal",
    "weekend_ot_start", "overtime_rate_weekend", "overtime_rate_holiday", "remind_sedentary_minutes", "focus_min_minutes"]) {
    const blank = raw[id] === "" || raw[id] == null;
    let invalid;
    if (id.endsWith("_start")) invalid = !blank && !/^(?:[01]\d|2[0-3]):[0-5]\d$/.test(raw[id]);
    else {
      const n = Number(raw[id]);
      invalid = blank ? !optional.includes(id) : !Number.isFinite(n) || n < 0;
      if (!blank && ["remind_sedentary_minutes", "focus_min_minutes"].includes(id)) {
        invalid = !Number.isInteger(n) || n < (id === "focus_min_minutes" ? 10 : 1) || n > 120;
      }
    }
    invalid = invalid || (raw.invalid_native || []).includes(id);
    // Valid drafts remain in queued snapshots even when a later action hides their field.
    if (invalid && !settingsFieldIsActive(id, active) && Object.prototype.hasOwnProperty.call(saved, id)) cfg[id] = saved[id];
  }
  return cfg;
}

let settingsManualDays = false;
let settingsManualDaysMonth = "";
let settingsWorkdays = null;
let settingsWorkdaysMonth = "";
let settingsWorkdaysManual = false;

function expireSettingsWorkdays() {
  if (settingsManualDays && settingsManualDaysMonth !== currentYearMonth()) setWorkdaysModeUI(false);
}

function settingsFormValues() {
  expireSettingsWorkdays();
  const values = { salary_mode: readSalaryMode(), workdays_manual: settingsManualDays, invalid_native: [] };
  document.querySelectorAll("#viewSettings input, #viewSettings select").forEach(input => {
    if (input.id) values[input.id] = input.type === "checkbox" ? input.checked : input.value;
    if (input.id && input.validity && input.validity.badInput) values.invalid_native.push(input.id);
  });
  if (!settingsManualDays || values.salary_mode === "hourly") values.workdays_override = "";
  return values;
}

let settingsStatusTimer = null;

function setSettingsSaveState(state, message) {
  clearTimeout(settingsStatusTimer);
  settingsStatusTimer = null;
  const target = $("settingsSaveStatus");
  if (!target) return;
  target.classList.remove("settings-status-faded");
  target.dataset.state = state;
  const labels = { loading: "正在加载", saving: "保存中…", saved: "已自动保存",
    dirty: "编辑后自动保存", error: "未保存，请重试", ready: "" };
  target.textContent = message ?? labels[state] ?? "";
  if (state === "saved") {
    settingsStatusTimer = setTimeout(() => {
      target.classList.add("settings-status-faded");
      settingsStatusTimer = null;
    }, 2000);
  }
}

function setWorkdaysModeUI(manual) {
  if (settingsManualDays && !manual) settingsWorkdays = null;
  settingsManualDays = !!manual;
  settingsManualDaysMonth = manual ? currentYearMonth() : "";
  $("workdaysManualFields").classList.toggle("hidden", !manual);
  document.querySelectorAll("#workdaysModeSeg button").forEach(button => {
    const active = (button.dataset.workdaysMode === "manual") === !!manual;
    button.classList.toggle("active", active);
    button.setAttribute("aria-pressed", String(active));
  });
  if (!manual) $("workdays_override").value = "";
}

function setSettingsWorkdays(count) {
  const number = Number(count);
  settingsWorkdays = Number.isFinite(number) && number > 0 ? number : null;
  settingsWorkdaysMonth = currentYearMonth();
  settingsWorkdaysManual = lastSaved ? settingsOverrideIsCurrent(JSON.parse(lastSaved), currentYearMonth()) : false;
  refreshSettingsUI();
}

function settingsPreviewWorkdays() {
  if (settingsManualDays) return Number($("workdays_override").value) || null;
  // A former manual count cannot be presented as an automatic calendar result.
  const saved = lastSaved ? JSON.parse(lastSaved) : {};
  if (settingsOverrideIsCurrent(saved, currentYearMonth()) || settingsWorkdaysManual) return null;
  return settingsWorkdaysMonth === currentYearMonth() ? settingsWorkdays : null;
}

function showSettingsFieldErrors(errors) {
  document.querySelectorAll("#viewSettings .settings-field-error").forEach(node => node.remove());
  document.querySelectorAll("#viewSettings [aria-invalid]").forEach(input => {
    input.removeAttribute("aria-invalid");
    input.removeAttribute("aria-errormessage");
  });
  errors.forEach(error => {
    const input = $(error.id);
    if (!input) return;
    const text = document.createElement("p");
    text.className = "settings-field-error";
    text.id = "settingsError_" + error.id;
    text.textContent = error.message;
    input.setAttribute("aria-invalid", "true");
    input.setAttribute("aria-errormessage", text.id);
    input.closest("label").after(text);
  });
}

function validateSettingsForm() {
  const cfg = settingsFormValues();
  const errors = settingsValidationErrors(cfg);
  document.querySelectorAll("#viewSettings input[type=number], #viewSettings input[type=time]").forEach(input => {
    if (settingsFieldIsActive(input.id, cfg) && !input.matches(":disabled") && !input.closest(".hidden") && input.validity.badInput && !errors.some(error => error.id === input.id)) {
      errors.push({ id: input.id, message: "请输入有效数值" });
    }
  });
  showSettingsFieldErrors(errors);
  return errors.length === 0;
}

function refreshSettingsUI() {
  const cfg = settingsFormValues();
  $("overtimeFields").disabled = !cfg.overtime_enabled;
  $("mealFields").classList.toggle("hidden", !cfg.overtime_meal_enabled);
  $("sedentaryFields").classList.toggle("hidden", !cfg.remind_sedentary_enabled);
  $("focusFields").classList.toggle("hidden", !cfg.focus_enabled);
  $("whitelistFields").disabled = !cfg.monitor_app_usage;
  $("whitelistEditor").classList.toggle("hidden", !cfg.app_whitelist_enabled);
  $("workdaysOverrideHint").textContent = "仅覆盖 " + currentYearMonth().replace("-", " 年 ") + " 月，下个月恢复自动计算";
  $("overtimeStartHint").textContent = cfg.overtime_start ? "按自定义起算时间计算" :
    "当前跟随下午下班时间：" + (cfg.pm_end || "请先设置作息");
  const rates = settingsRateModel(cfg);
  $("weekendRateHint").textContent = cfg.overtime_rate_weekend === "" ?
    "当前沿用工作日：" + fmtMoney(rates.weekend) + " / 小时" : "使用独立休息日费率";
  $("holidayRateHint").textContent = cfg.overtime_rate_holiday === "" ?
    "当前沿用休息日：" + fmtMoney(rates.holiday) + " / 小时" : "使用独立节假日费率";
  $("offworkReminderHint").textContent = "按下午下班时间提醒：" + (cfg.pm_end || "尚未设置");
  $("paydayReminderHint").textContent = "每月 " + (cfg.payday || "已保存的发薪日") + " 日发送一次上月战绩";
  const count = readWhitelist().length;
  const empty = cfg.app_whitelist_enabled && count === 0;
  $("whitelistStatus").textContent = !cfg.monitor_app_usage ? "应用监控已关闭，名单暂不生效" :
    !cfg.app_whitelist_enabled ? "当前记录全部前台应用" : empty ?
      "名单为空：当前仍会记录全部前台应用" : "当前仅记录名单中的 " + count + " 个应用";
  $("whitelistStatus").classList.toggle("settings-warning", cfg.monitor_app_usage && empty);
  $("retentionHint").textContent = Number(cfg.retention_days) === 0 ? "保留全部历史，整理时不因保留期删除记录" :
    "整理时会删除超过 " + cfg.retention_days + " 天的历史记录，建议先备份";
  $("retentionHint").classList.toggle("settings-warning", Number(cfg.retention_days) > 0);
  const days = settingsPreviewWorkdays();
  const model = settingsScheduleModel(cfg, days);
  $("scheduleSummary").textContent = model.valid ? "每天计薪 " + model.hours.toFixed(1) + " 小时 · 午休不计入" : model.message;
  const preview = $("salaryPreview");
  preview.textContent = model.message;
  if (model.valid && model.hourlyRate !== null) {
    preview.textContent = cfg.salary_mode === "hourly" ? "按时薪 × 工作时长计算" : "本月 " + days + " 个工作日";
    const line = document.createElement("div");
    line.append("参考时薪 ");
    const hourly = document.createElement("strong");
    hourly.textContent = fmtMoney(model.hourlyRate);
    const daily = document.createElement("strong");
    daily.textContent = fmtMoney(model.dailyPay);
    line.append(hourly, " · 每日满勤 ", daily);
    preview.append(line);
  }
  if (settingsWorkdaysMonth === currentYearMonth() && days) {
    $("workdaysInfo").textContent = currentYearMonth().replace("-", " 年 ") + " 月 · " + days + " 天 · " + (settingsManualDays ? "手动设置" : "自动计算");
  } else {
    $("workdaysInfo").textContent = settingsManualDays ? "请填写本月上班天数" : "正在获取本月工作日…";
  }
  $("durationPreview").textContent = { hms: "5小时10分30秒", hm: "5小时10分", h: "5.2h" }[cfg.duration_format] || "5小时10分30秒";
  const style = cfg.tagline_style;
  const dynamic = lastStatus ? dynamicTagline(lastStatus) : "正在获取当前状态…";
  $("taglinePreview").textContent = style === "none" ? "副标题已隐藏" : style === "dynamic" ? dynamic :
    style === "custom" ? String(cfg.tagline_custom || "").trim() || dynamic : FIXED_TAGLINES[style] || dynamic;
  $("taglineCounter").textContent = String(cfg.tagline_custom || "").length + " / 40 字";
  document.querySelectorAll("#sedentaryPresets button").forEach(button => {
    const active = button.dataset.minutes === cfg.remind_sedentary_minutes;
    button.classList.toggle("active", active);
    button.setAttribute("aria-pressed", String(active));
  });
}

function initSettingsUI() {
  const view = $("viewSettings");
  const nav = [...document.querySelectorAll(".settings-nav button")];
  const sectionTop = card => card.getBoundingClientRect().top - view.getBoundingClientRect().top;
  const headerHeight = () => view.querySelector(".settings-head").offsetHeight;
  const select = button => nav.forEach(item => {
    item.classList.toggle("active", item === button);
    if (item === button) item.setAttribute("aria-current", "location");
    else item.removeAttribute("aria-current");
  });
  nav.forEach(button => button.addEventListener("click", () => {
    select(button);
    view.scrollTo({ top: view.scrollTop + sectionTop($(button.dataset.settingsSection)) - headerHeight() - 12,
      behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
  }));
  view.addEventListener("scroll", () => {
    const past = nav.filter(button => sectionTop($(button.dataset.settingsSection)) <= headerHeight() + 20);
    select(past[past.length - 1] || nav[0]);
  }, { passive: true });
  view.addEventListener("input", () => {
    if (configLoaded) { validateSettingsForm(); setSettingsSaveState("dirty"); }
    refreshSettingsUI();
  });
  view.addEventListener("change", refreshSettingsUI);
  document.querySelectorAll("#workdaysModeSeg button").forEach(button => button.addEventListener("click", () => {
    const manual = button.dataset.workdaysMode === "manual";
    if (manual && !$("workdays_override").value && settingsPreviewWorkdays()) {
      $("workdays_override").value = settingsPreviewWorkdays();
    }
    setWorkdaysModeUI(manual);
    refreshSettingsUI();
    if (!manual || $("workdays_override").value) saveNow();
    else { $("workdays_override").focus(); setSettingsSaveState("dirty", "请填写本月上班天数"); }
  }));
  document.querySelectorAll("#sedentaryPresets button").forEach(button => button.addEventListener("click", () => {
    $("remind_sedentary_minutes").value = button.dataset.minutes;
    refreshSettingsUI();
    saveNow();
  }));
  $("billStyleSeg").addEventListener("keydown", event => {
    if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)) return;
    const buttons = [...document.querySelectorAll("#billStyleSeg button")];
    const index = buttons.indexOf(document.activeElement);
    if (index < 0) return;
    event.preventDefault();
    const next = buttons[(index + (event.key === "ArrowLeft" || event.key === "ArrowUp" ? -1 : 1) + buttons.length) % buttons.length];
    next.focus();
    next.click();
  });
}
