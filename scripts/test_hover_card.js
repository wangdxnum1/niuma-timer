// 悬停卡片（hover_card.html）视觉回归测试：
// 锁住两个曾塌方的 CSS 契约，避免「改着改着又看不见」：
// 1) 时间轴 .taxis 必须是 flex 横向一条轴（修复前缺 display:flex，5 个块级 .tseg
//    竖向堆叠成「多条进度条」）；
// 2) 大字金额 .amount 必须用纯金实心色（color:#ffd650），绝不能用
//    background-clip:text + -webkit-text-fill-color:transparent —— 悬停卡窗口是
//    .transparent(true) 透明 WebView，裁剪渐变文字在透明表面下会渲染成不可见
//    （主窗口不透明所以正常）。这是「大字金额消失」的根因。
// 3) 休息日 .card.rest .amount 同步去掉裁剪，改置灰 color。
// 4) 大字金额所在的 .hero 必须 flex:0 0 auto（不参与收缩）。.card 是 flex 列容器，
//    .hero 是它唯一带 overflow:hidden 的子项——按 flex 规范其自动最小尺寸为 0，
//    内容一超高，全部溢出量就被它独自吸收压扁，34px 金额只剩 1px 被裁没
//    （2026-09-14 二次返工查出的真根因，比第 2 条更底层）。
// 5) 窗口高度两侧必须一致：hover_card.html 的 body{height} == tray.rs HOVER_CARD_H - 16，
//    且总高度够装下全部内容。任何一侧单独改动都会让金额再次消失。
// 运行：node scripts/test_hover_card.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const html = fs.readFileSync(
  path.join(ROOT, "frontend", "hover_card.html"),
  "utf8"
);

let pass = 0;
let fail = 0;
function eq(label, actual, expect) {
  if (actual === expect) {
    pass++;
    console.log("  PASS " + label + "  ->  " + actual);
  } else {
    fail++;
    console.log("  FAIL " + label + "  ->  " + actual + "  (期望 " + expect + ")");
  }
}
function has(label, src, needle) {
  eq(label, src.includes(needle), true);
}
function lacks(label, src, needle) {
  eq(label, src.includes(needle), false);
}

// 抽取 CSS 规则块做精确断言。selRe 必须能唯一定位到「目标选择器起始处」，
// 否则会像 `.amount` 被 `.card.rest .amount` 抢先匹配那样取错块。
function ruleBlockRe(src, selRe) {
  const m = src.match(selRe);
  if (!m) return null;
  const start = m.index;
  const j = src.indexOf("}", start);
  if (j < 0) return null;
  return src.slice(start, j);
}

const taxis = ruleBlockRe(html, /\n\s*\.taxis\s*\{/);
// 注意：独立 .amount 必须排除 .card.rest .amount，故用行首空白锚定
const amount = ruleBlockRe(html, /\n[ \t]*\.amount\s*\{/);
const restAmount = ruleBlockRe(html, /\.card\.rest \.amount\s*\{/);

// ---------------------------------------------------- 1. 时间轴必须是横向一条
eq("存在 .taxis 规则", taxis !== null, true);
has("时间轴用 flex 横向排布（修复多条进度条）", taxis, "display: flex");
has("时间轴子段拉伸填满高度", taxis, "align-items: stretch");

const tseg = ruleBlockRe(html, /\n\s*\.tseg\s*\{/);
eq("存在 .tseg 规则", tseg !== null, true);
has("时间轴分段不被 flex 压缩", tseg, "flex-shrink: 0");

// ---------------------------------------------------- 2. 金额必须可靠可见
eq("存在 .amount 规则", amount !== null, true);
has("金额用纯金实心色（透明 WebView 下一定可见）", amount, "color: #ffd650");
lacks("金额不再用 background-clip 裁剪（透明表面会不可见）", amount, "background-clip: text");
lacks("金额不再用透明文字填充", amount, "-webkit-text-fill-color: transparent");
has("金额保留轻微阴影增加立体感", amount, "text-shadow");

// ---------------------------------------------------- 3. 休息日金额同步去掉裁剪
eq("存在 .card.rest .amount 规则", restAmount !== null, true);
has("休息日金额置灰", restAmount, "color: #6a6a70");
lacks("休息日金额不再用透明填充", restAmount, "-webkit-text-fill-color");

// ---------------------------------------------------- 4. 结构上：全天进度条唯一 + 时间轴单轴
eq("全天进度条 lp 仅一个容器", (html.match(/class="lp[ "]/g) || []).length, 1);
eq("时间轴 taxis 仅一个容器", (html.match(/class="taxis"/g) || []).length, 1);
has("金额挂载点 #earned 存在", html, 'id="earned"');
has("时间轴现在指针 #tlNow 存在", html, 'id="tlNow"');

// ---------------------------------------------------- 5. 缓存戳由 build.rs 自动 bust
// （hover_card.html 内容一变，build.rs 会把 tray.rs 的 ?v= 改成内容指纹，
//  不依赖手工改版本号；此处仅做软性提示，不阻断）
const traySrc = fs.readFileSync(
  path.join(ROOT, "src-tauri", "src", "tray.rs"),
  "utf8"
);
has("tray.rs 引用 hover_card.html（缓存戳由 build.rs 管理）", traySrc, "hover_card.html?v=");
const buildSrc = fs.readFileSync(
  path.join(ROOT, "src-tauri", "build.rs"),
  "utf8"
);
has("build.rs 把 src/tray.rs 纳入缓存戳同步目标", buildSrc, '"src/tray.rs"');

// --------------------------------- 6. 金额块不可收缩 + 窗口/CSS 尺寸契约（防再次返工）
// 这是「大字金额消失」最底层的根因：flex 列容器里唯一 overflow:hidden 的子项
// （.hero）自动最小尺寸退化为 0，内容超高时被它独自吸收，金额被裁得只剩 1px。
const hero = ruleBlockRe(html, /\n[ \t]*\.hero\s*\{/);
eq("存在 .hero 规则", hero !== null, true);
has("hero 不参与 flex 收缩（否则金额会被压扁裁掉）", hero, "flex: 0 0 auto");
has(
  "卡片是 flex 列容器（收缩机制的前提，勿改）",
  ruleBlockRe(html, /\n[ \t]*\.card\s*\{/),
  "flex-direction: column"
);

// 两侧尺寸契约：窗口高（tray.rs）必须等于 CSS 的 body 高。body 自带上下各 8px
// padding（box-sizing:border-box），故卡片实际高 = 窗口高 - 16 = 336。
// body 的 height 紧邻 padding: 8px 出现，借此唯一定位（html,body 那条规则无 height）。
const mBodyH = html.match(/height:\s*(\d+)px;\s*\n\s*padding:\s*8px;/);
const mRustH = traySrc.match(/const HOVER_CARD_H: f64 = ([\d.]+);/);
eq("能解析到 CSS body 高度", mBodyH !== null, true);
eq("能解析到 tray.rs HOVER_CARD_H", mRustH !== null, true);
if (mBodyH && mRustH) {
  eq(
    "body 高度 = HOVER_CARD_H（窗口与 CSS 不脱节）",
    Number(mBodyH[1]),
    Number(mRustH[1])
  );
  // 内容实测需要 ~305px（2026-09-14 按真实渲染逐块量过：head 17 + hero 34 + quips 14
  // + lp 20 + c1 14 + tl 23 + grid 90 + otrow 29 + foot 15 + 8 个 6px 间隙 48），
  // 再加卡片 padding 22 与 body padding 16，窗口低于 344 就装不下、金额必被挤扁。
  eq("窗口高度留有装下全部内容的余量（>= 344）", Number(mRustH[1]) >= 344, true);
}

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
