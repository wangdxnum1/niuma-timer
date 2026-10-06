const DETAIL_VIEWS = ["viewOt", "viewAct", "viewApp", "viewAudio"];
let lastDetailView = "viewOt";

// 视图切换统一入口：主界面 / 设置 / 4 个明细分段互斥（只留一个 .app 可见）
function showView(id) {
  const target = $(id);
  if (!target) return; // 目标视图不存在则不操作，避免误隐藏所有视图
  if (curView === "viewSettings" && id !== "viewSettings") {
    if (id === "viewUpdate") updateSettingsScroll = $("viewSettings").scrollTop;
    saveIfChanged(); // 自定义副标题等可能还没失焦
  }
  document.querySelectorAll(".app").forEach((v) => v.classList.add("hidden"));
  target.classList.remove("hidden");
  curView = id;
  // 导航同步（契约见 scripts 目录的导航回归测试）：
  // 侧栏高亮——4 个明细视图都映射到「明细」聚合项，设置项在设置视图点亮；
  // 翻页器——明细 / 账单各一条，仅在对应视图显示，页名与圆点跟随当前页
  const isDetail = DETAIL_VIEWS.includes(id);
  if (isDetail) lastDetailView = id;
  const navKey = isDetail ? "detail" : id;
  // 更新页不在侧栏里（由设置页/托盘/公告进入），高亮挂到「设置」，免得整排都不亮
  const railKey = id === "viewUpdate" ? "viewSettings" : navKey;
  document.querySelectorAll(".rail-item").forEach((b) => {
    b.classList.toggle("active", b.dataset.nav === railKey);
  });
  $("detailPager").classList.toggle("hidden", !isDetail);
  $("billPager").classList.toggle("hidden", id !== "viewBill");
  if (isDetail) {
    const names = {
      viewOt: "加班明细",
      viewAct: "键鼠明细",
      viewApp: "应用明细",
      viewAudio: "媒体明细",
    };
    $("pgName").textContent = names[id];
  }
  document.querySelectorAll("#detailPager .pg-dot").forEach((b) => {
    b.classList.toggle("active", b.dataset.nav === id); // 账单翻页器圆点由 setBillTabUI 同步
  });
  if (id === "viewMain") resetHistDates();
  if (id === "viewBill") {
    setBillTabUI(curBillTab); // 翻页器页名/圆点与记忆的页保持同步
    loadBillTab(); // 懒渲染：进账单页才拉当前页（周级聚合不进 tick）
  }
  // 更新页没有轮询：进页时先出已有内容/公告，没有公告才联网查一次
  if (id === "viewUpdate") {
    if (updateAnnounce) paintUpdate();
    else loadUpdateInfo();
    paintUpdateProgress();
  }
  // 懒渲染下目标视图可能从未画过（或还停留在上次的数据），立刻补一次，
  // 否则要等下一个轮询周期才出内容。
  repaintCurrentView();
}

// 用缓存立即补画当前视图；缓存还没到就现拉一次，避免切过去看到空白
function repaintCurrentView() {
  if (curView === "viewSettings") refreshSettingsUI();
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
    if (!btn.dataset.mon) return; // 设置页账单风格分段复用此类名，非监控分段不处理
    curMon = btn.dataset.mon;
    document.querySelectorAll("#monitorCard .mon-seg-item").forEach((b) => {
      b.classList.toggle("active", b === btn);
    });
    Object.entries(MON_PANES).forEach(([key, id]) =>
      $(id).classList.toggle("hidden", key !== curMon)
    );
  });
});
$("monDetailBtn").addEventListener("click", () => showView(MON_VIEWS[curMon]));
// 摸鱼统计：点应用行的分类标签循环改分类（事件委托——列表每 2 秒重渲染不丢监听）
$("appuList").addEventListener("click", (e) => {
  const sug = e.target.closest(".cat-suggest");
  if (sug) {
    acceptSuggestion(sug.dataset.sapp, sug.dataset.scat);
    return;
  }
  const chip = e.target.closest(".cat-chip");
  if (chip) cycleAppCategory(chip.dataset.app, chip.dataset.cat);
});
// 明细页的日期导航：翻天后立即按新日期重新拉取
$("actPrevDay").addEventListener("click", () => shiftHist("act", -1, loadActivity));
$("actNextDay").addEventListener("click", () => shiftHist("act", 1, loadActivity));
$("appuPrevDay").addEventListener("click", () => shiftHist("appu", -1, loadAppUsage));
$("appuNextDay").addEventListener("click", () => shiftHist("appu", 1, loadAppUsage));
$("audioPrevDay").addEventListener("click", () => shiftHist("audio", -1, loadAudioUsage));
$("audioNextDay").addEventListener("click", () => shiftHist("audio", 1, loadAudioUsage));

// ---- 本周打工账单（viewBill：进页拉一次，不接 tick 轮询——周级聚合）----

// 周损味金句四档，按周摸鱼率降序取档；未配月薪时过滤掉提钱的 ≥40% 档（纯摸鱼率版）
$("billPrevWeek").addEventListener("click", () => shiftWeek(1));
$("billExportBtn").addEventListener("click", exportWeekBillCsv);
$("billImageBtn").addEventListener("click", saveBillImage);
$("reportImageBtn").addEventListener("click", saveReportImage);
$("billRetryBtn").addEventListener("click", loadWeekBill);
$("reportRetryBtn").addEventListener("click", loadMonthlyReport);
$("billNextWeek").addEventListener("click", () => shiftWeek(-1));
// 时间线日导航（v1.6.0）：‹ 往过去翻无上限，› 往未来翻封顶今天
$("tlPrevDay").addEventListener("click", () => tlShift(1));
$("tlNextDay").addEventListener("click", () => tlShift(-1));
document.querySelectorAll("#billStyleSeg .bill-style-card").forEach((b) => {
  b.addEventListener("click", () => {
    setBillStyleUI(b.dataset.bill);
    saveIfChanged();
    paintWeekBill();
  });
});

// 调试模式彩蛋：2 秒内连点设置按钮 5 次开关（进程级，不落盘，重启失效）
let debugOn = false;
let eggClicks = 0;
let eggTimer = 0;

function toggleDebug() {
  debugOn = !debugOn;
  const card = $("debugCard");
  card.classList.toggle("hidden", !debugOn);
  // 卡片在长设置页最底部，仅显隐不足以让用户看到：开启时滚入视野（block:nearest
  // 已可见则不动，避免无谓跳动）
  if (debugOn) card.scrollIntoView({ block: "nearest" });
  showToast(debugOn ? "调试模式已开启" : "调试模式已关闭", "ok");
}

// 侧边导航栏 + 明细翻页器：任意视图直达（取代原「‹ 返回」的网页式导航）。
// 自动保存（离开设置页）与历史日期复位（回主页）仍由 showView 统一处理
document.querySelectorAll(".rail-item").forEach((btn) => {
  btn.addEventListener("click", () => {
    // 调试彩蛋：设置按钮 2 秒内连点 5 次翻转调试模式（滑动窗口，每击重置计时）
    if (btn.dataset.nav === "viewSettings") {
      eggClicks++;
      clearTimeout(eggTimer);
      eggTimer = setTimeout(() => (eggClicks = 0), 2000);
      if (eggClicks >= 5) {
        eggClicks = 0;
        toggleDebug();
      }
    }
    showView(btn.dataset.nav === "detail" ? lastDetailView : btn.dataset.nav);
  });
});

// 明细翻页器：‹ › 循环切页（加班→键鼠→应用→媒体→加班），圆点直达任意页
function pgStep(delta) {
  const i = DETAIL_VIEWS.indexOf(curView);
  if (i < 0) return;
  showView(DETAIL_VIEWS[(i + delta + DETAIL_VIEWS.length) % DETAIL_VIEWS.length]);
}
$("pgPrev").addEventListener("click", () => pgStep(-1));
$("pgNext").addEventListener("click", () => pgStep(1));
document.querySelectorAll("#detailPager .pg-dot").forEach((btn) => {
  btn.addEventListener("click", () => showView(btn.dataset.nav));
});

// 滚轮翻页：明细内容滚到头（顶部/底部）再滚即翻页，500ms 冷却防触控板惯性
// 一口气连翻。没到边界时滚轮照常滚动内容，长表格不受影响
let wheelFlipUntil = 0;
DETAIL_VIEWS.forEach((vid) => {
  $(vid).addEventListener(
    "wheel",
    (e) => {
      if (Date.now() < wheelFlipUntil) return;
      const el = e.currentTarget;
      const atTop = el.scrollTop <= 0;
      const atBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - 1;
      if (!atTop && !atBottom) return;
      const i = DETAIL_VIEWS.indexOf(curView);
      if (i < 0) return;
      if (e.deltaY > 0 && atBottom) {
        wheelFlipUntil = Date.now() + 500;
        showView(DETAIL_VIEWS[(i + 1) % DETAIL_VIEWS.length]);
      } else if (e.deltaY < 0 && atTop) {
        wheelFlipUntil = Date.now() + 500;
        showView(DETAIL_VIEWS[(i + DETAIL_VIEWS.length - 1) % DETAIL_VIEWS.length]);
      }
    },
    { passive: true }
  );
});

// 账单翻页器：与明细同款交互——‹ › 循环切洞察页（账单→热力→趋势→身体→账单），
// 圆点直达；滚轮翻页与明细共用 wheelFlipUntil 冷却（两条翻页器不同时可见）
function billStep(delta) {
  const i = BILL_TAB_KEYS.indexOf(curBillTab);
  if (i < 0) return;
  setBillTabUI(BILL_TAB_KEYS[(i + delta + BILL_TAB_KEYS.length) % BILL_TAB_KEYS.length]);
  loadBillTab();
}
$("billPgPrev").addEventListener("click", () => billStep(-1));
$("billPgNext").addEventListener("click", () => billStep(1));
document.querySelectorAll("#billPager .pg-dot").forEach((btn) => {
  btn.addEventListener("click", () => {
    setBillTabUI(btn.dataset.btab);
    loadBillTab();
  });
});
$("viewBill").addEventListener(
  "wheel",
  (e) => {
    if (Date.now() < wheelFlipUntil) return;
    const el = e.currentTarget;
    const atTop = el.scrollTop <= 0;
    const atBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - 1;
    if (!atTop && !atBottom) return;
    const i = BILL_TAB_KEYS.indexOf(curBillTab);
    if (i < 0) return;
    if (e.deltaY > 0 && atBottom) {
      wheelFlipUntil = Date.now() + 500;
      billStep(1);
    } else if (e.deltaY < 0 && atTop) {
      wheelFlipUntil = Date.now() + 500;
      billStep(-1);
    }
  },
  { passive: true }
);

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
  // 关键证据：记录 WebView2 实际加载的 URL（?v=3c970508 = 新前端；旧值 = 缓存没刷新）
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

// 调试卡按钮组：直发通知验证通道 / 重置提醒状态 / 按真实规则立即调度一次。
// 点击给 toast 反馈——系统通知可能被勿扰模式吞掉，前端确认能区分「命令没发出去」
// 与「发出去了但系统没弹」；失败一律 flog 落 debug.log（命令层失败不再静默）。
// （调试卡由彩蛋开关：2 秒内连点设置按钮 5 次显隐，进程级不落盘）
function bindDebugBtn(id, cmd, label, okMsg) {
  $(id).addEventListener("click", () => {
    invoke(cmd)
      .then(() => showToast(okMsg || `${label}：已执行`, "ok"))
      .catch((e) => {
        flog(`${label}失败: ${e}`);
        showToast(`${label}失败，详见日志`, "err");
      });
  });
}
bindDebugBtn("testOffworkBtn", "test_offwork_notify", "下班提醒");
bindDebugBtn("testSedentaryBtn", "test_sedentary_notify", "休息提醒");
bindDebugBtn("testResetBtn", "reset_remind_state", "重置提醒状态");
bindDebugBtn("testTickBtn", "run_remind_tick", "立即调度", "已调度：满足条件才会弹");
// 「模拟久坐」：只伪造连续活跃起点，随后走真实 tick——不用等阈值分钟数
bindDebugBtn("testSedentaryTriggerBtn", "test_sedentary_trigger", "模拟久坐", "已模拟并调度：满足条件才会弹");


initSettingsUI();
boot();
watchVisibility();
watchUpdateView();
watchUpdateProgress();
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

document.querySelectorAll("#billSpanSeg .mon-seg-item").forEach((b) => {
  b.addEventListener("click", () => {
    setBillSpanUI(b.dataset.span);
    weekOffset = 0; // 切跨度重置偏移：上一期的语义随跨度变化
    saveIfChanged({ silent: true }); // 记住浏览偏好，不打断浏览；失败仍提示
    setBillTabUI(curBillTab); // 报告页页名随月/年切换（月报 ↔ 年报）
    loadBillTab();
  });
});
