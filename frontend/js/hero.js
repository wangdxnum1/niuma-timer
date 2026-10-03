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
    renderSlackBurn();
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

// 状态徽章：主页品牌行右侧的动态状态（搬砖中 / 已下班 / 今天休息）。
function renderBadge(s) {
  const badge = $("statusBadge");
  if (s.paused) {
    badge.textContent = "已暂停 · 钱先冻结";
    badge.className = "badge off";
  } else if (!s.is_workday) {
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

