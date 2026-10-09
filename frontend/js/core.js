// 牛马计时器 主界面前端逻辑
// window.__TAURI__ 由 Rust 端 append_invoke_initialization_script 注入的垫片暴露
const TAURI = window.__TAURI__;
const invoke = TAURI.core.invoke;

// 前端版本标记：写进每条日志，用于核对 WebView2 实际加载的是哪个版本（防旧缓存）
const FE_VER = "v351e5f26";

// 主窗口是否可见。托盘常驻期间窗口是 hide 的，此时前端一切轮询都没意义
// （界面看不见，数据看不见），由 Rust 端 1s 线程广播 win-visibility 驱动。
// 初值 true：万一事件系统不通，退化为「始终轮询」的旧行为，不会更差。
let winVisible = true;

// 当前显示的视图 id。主界面与二级页互斥（showView 只留一个 .app 可见），
// 所以「看不见的视图」根本不必重绘——此前每 2 秒会把三个二级页的图表、
// 长列表全量重建一遍，而这些 DOM 正隐藏在主界面背后，纯属浪费。
// 初值 viewMain：万一状态维护失效，退化为「只画主界面」，不会白屏。
let curView = "viewMain";

// 最近一次拉到的监控数据。懒渲染后切进二级页要靠它立即补画，
// 否则得等下一个轮询周期才出内容。
const viewData = { activity: null, appu: null, audio: null };

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

// 未处理的 Promise rejection 同样进日志（hover_card 一直有同款，主窗口此前漏配：
// saveBillImage 等异步链抛错时按钮复位但无 toast 无日志）
window.addEventListener("unhandledrejection", function (ev) {
  var r = ev.reason;
  flog("JS REJECTION: " + (r && r.message ? r.message : String(r)));
});

// 像素 → 距离（96dpi 估算）单一实现：明细页与身体账单共用，单位档位统一
// （米一位小数，≥1km 显 km）。此前 monitor/insights 各一套，同一数据两处口径分裂。
function fmtDist(px) {
  const m = ((Number(px) || 0) * 2.54) / 96 / 100;
  if (m >= 1000) return (m / 1000).toFixed(2) + " km";
  return m.toFixed(1) + " m";
}

// 秒数 → "x小时x分 / x分钟 / x秒" 单一实现（insights 时间线/专注汇总共用）。
// 此前 insights 的 tlDur 与这里的措辞漂移（"59分钟" vs "60分"）。
function fmtDurCN(sec) {
  sec = Math.max(0, Math.round(sec || 0));
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  if (h > 0) return h + "小时" + (m > 0 ? m + "分" : "");
  if (m > 0) return m + "分钟";
  return sec + "秒";
}

// 金额单一口径：¥ + 两位小数（此前 17 处内联 toFixed(0)/toFixed(2) 漂移，
// 同一「全天应赚」主界面 ¥123.45、悬停卡 ¥123）。hover_card 是独立文档，
// 自持一份同体实现，由 test_hover_card 断言逐字一致。
function fmtMoney(n) {
  return "¥" + (Number(n) || 0).toFixed(2);
}

// 摸鱼换算单位表：key → 量词与单价。custom 的单价走配置 slack_equiv_price。
// 纯前端消费，config.rs 只存 key 与自定义单价。
const SLACK_UNITS = {
  milktea: { label: "杯奶茶", price: 15 },
  coffee: { label: "杯咖啡", price: 30 },
  takeout: { label: "顿外卖", price: 25 },
  movie: { label: "张电影票", price: 40 },
  custom: { label: "份快乐", price: 0 },
};

// 摸鱼成本 → 人话换算："≈2.6 杯奶茶"。关闭/非正成本/非法单价返回空串。
// 精度：floor 到 0.1 档，不足 0.1 显示「不足 0.1」不显示 0（0 看起来像没摸鱼）。
function fmtSlackEquiv(cost, unitKey, customPrice) {
  const unit = SLACK_UNITS[unitKey];
  if (!unit || !(cost > 0)) return "";
  const price = unitKey === "custom" ? Number(customPrice) : unit.price;
  if (!(price > 0)) return "";
  const n = Math.floor((cost / price) * 10) / 10;
  return "≈" + (n < 0.1 ? "不足 0.1" : String(n)) + " " + unit.label;
}

const $ = (id) => document.getElementById(id);
