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
    renderSlackBurn(); // 停用时 hero 烧钱行一并隐藏
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
    if (home) {
      renderAppRows(home, apps, 6);
      // 摸鱼速览：主页监控卡里一行「工作 X · 摸鱼 Y」（应用构成切片）
      const cats = s.categories || [];
      const w = cats.find((c) => c.key === "work");
      const sl = cats.find((c) => c.key === "slack");
      if (apps.length && (w || sl)) {
        home.insertAdjacentHTML(
          "afterbegin",
          '<div class="mon-slack"><span class="cat-dot cat-c-work"></span>工作 ' +
            fmtDurCN(w ? w.seconds : 0) +
            '<span class="cat-dot cat-c-slack"></span>摸鱼 ' +
            fmtDurCN(sl ? sl.seconds : 0) +
            "</div>"
        );
      }
    }
    return;
  }
  if (curView !== "viewApp") return;
  updateDayNav("appu");
  renderCategories(s);
  const list = $("appuList");
  if (list) renderAppRows(list, apps, 0, dayEmpty(s.date, "应用使用记录"), { chips: true });
  renderHourChart($("appuChart"), s.hourly || [], isToday(s.date));
}

// ---- 摸鱼统计：分类渲染与改分类 ----

// 摸鱼率 → 损味文案四档（阈值边界：10 / 25 / 40，纯函数便于断言）
const SLACK_QUIPS = [
  [10, "老板的梦中情马"],
  [25, "摸得克制，装得敬业"],
  [40, "快三分之一的班白上了"],
  [101, "老板看完连夜注销公司"],
];
function slackQuip(pct) {
  for (const [ceil, q] of SLACK_QUIPS) {
    if (pct < ceil) return q;
  }
  return SLACK_QUIPS[SLACK_QUIPS.length - 1][1];
}

// hero 卡烧钱行：摸鱼类时长 × 时薪。数据源 = 今日 app usage 的 categories +
// 最近一次状态（时薪/工作日），零新 IPC。休息日、无薪、无任何应用记录时整块隐藏；
// 摸鱼为 0 时显示 ¥0.00 不隐藏（正向激励）。由 tick() 与 paintAppUsage() 共同触发
function renderSlackBurn() {
  const box = $("slackBurn");
  if (!box) return;
  const st = lastStatus;
  const appu = viewData.appu;
  const cats = appu && appu.date && isToday(appu.date) ? appu.categories || [] : [];
  const total = cats.reduce((a, c) => a + (c.seconds || 0), 0);
  if (!st || !st.is_workday || !(st.hourly_rate > 0) || total <= 0) {
    box.classList.add("hidden");
    return;
  }
  const slackSec = (cats.find((c) => c.key === "slack") || {}).seconds || 0;
  const cost = (slackSec / 3600) * st.hourly_rate;
  const pct = (slackSec / total) * 100;
  box.classList.remove("hidden");
  $("sbAmt").textContent = "¥" + cost.toFixed(2);
  $("sbRate").textContent = "摸鱼率 " + Math.round(pct) + "%";
  $("sbQuip").textContent = slackQuip(pct);
}

// 分类循环顺序（点标签按此序切换）；key 为前端配色锚点
const CAT_CYCLE = ["工作", "摸鱼", "沟通", "其他"];
function catKey(label) {
  return { 工作: "work", 摸鱼: "slack", 沟通: "comm" }[label] || "other";
}

// 应用构成：堆叠条 + 四格时长（工作/摸鱼/沟通/其他）
function renderCategories(s) {
  const wrap = $("catSummary");
  if (!wrap) return;
  const cats = s.categories || [];
  const total = cats.reduce((a, c) => a + c.seconds, 0);
  const bar = cats
    .map(
      (c) =>
        c.seconds > 0
          ? '<i class="cat-c-' +
            catKey(c.label) +
            '" style="width:' +
            (total > 0 ? (c.seconds / total) * 100 : 0) +
            '%"></i>'
          : ""
    )
    .join("");
  // 构成金额化：工作日有时薪时，每格时长下方补该类折算金额；摸鱼类红色加粗，
  // 其余灰色小字（与 hero 烧钱行同口径：秒 ÷ 3600 × 时薪）
  const rate = lastStatus && lastStatus.is_workday ? lastStatus.hourly_rate : 0;
  const cells = cats
    .map((c) => {
      const key = catKey(c.label);
      const money =
        rate > 0
          ? '<span class="cat-money cat-money-' +
            key +
            '">¥' +
            ((c.seconds / 3600) * rate).toFixed(2) +
            "</span>"
          : "";
      return (
        '<div class="cat-cell"><span class="k">' +
        c.label +
        '</span><span class="v">' +
        fmtDurCN(c.seconds) +
        "</span>" +
        money +
        "</div>"
      );
    })
    .join("");
  wrap.innerHTML =
    '<div class="cat-bar">' + bar + '</div><div class="cat-list">' + cells + "</div>";
}

// 点分类标签循环改分类：覆盖写 config.app_categories（展示名 → 分类），
// 后端查询时归类——改完历史数据即时重新归类，无需迁移。
// 全链路 flog 打点：save 静默失败无从排查（曾实测「标签变了但 config 没落盘」）
async function cycleAppCategory(app, current) {
  flog("cat: click app=" + app + " cur=" + current);
  try {
    const cfg = await invoke("load_config");
    const map = Object.assign({}, cfg.app_categories || {});
    flog("cat: load_config ok, map=" + JSON.stringify(map));
    const idx = CAT_CYCLE.indexOf(current);
    map[app] = CAT_CYCLE[(idx + 1 + CAT_CYCLE.length) % CAT_CYCLE.length];
    await invoke("save_config", { cfg: { app_categories: map } });
    flog("cat: save_config ok, " + app + "=" + map[app]);
    showToast("已归类为「" + map[app] + "」", "ok");
    loadAppUsage();
  } catch (e) {
    flog("cat: FAILED " + e);
    showToast("分类保存失败：" + e, "err");
  }
}

// 应用行：图标 + 软件名 + 分类标签（明细页）+ 进度条 + 时长（按时长降序）。
// emptyText 自定义空态文案；opts.chips 为 true 时行内带可点的分类标签
function renderAppRows(container, apps, limit, emptyText, opts) {
  const chips = !!(opts && opts.chips);
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
    const chip =
      chips && a.category
        ? '<span class="cat-chip cat-c-' +
          catKey(a.category) +
          '" data-app="' +
          escapeHtml(a.app) +
          '" data-cat="' +
          escapeHtml(a.category) +
          '" title="点击切换分类">' +
          escapeHtml(a.category) +
          "</span>"
        : "";
    row.innerHTML =
      appIconHTML(a) +
      '<span class="tk-key appu-name" title="' + escapeHtml(a.app) + '">' + escapeHtml(a.app) + "</span>" +
      chip +
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
// ---- 开机自启（独立读写注册表，不进 config）----
