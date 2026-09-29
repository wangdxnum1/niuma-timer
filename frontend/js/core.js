// 牛马计时器 主界面前端逻辑
// window.__TAURI__ 由 Rust 端 append_invoke_initialization_script 注入的垫片暴露
const TAURI = window.__TAURI__;
const invoke = TAURI.core.invoke;

// 前端版本标记：写进每条日志，用于核对 WebView2 实际加载的是哪个版本（防旧缓存）
const FE_VER = "v3c970508";

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

const $ = (id) => document.getElementById(id);
