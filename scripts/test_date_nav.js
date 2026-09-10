// 历史日期导航的回归测试：
// 1) 从 app.js 抽出真实的日期工具函数跑边界（跨月、跨年、闰日、今天/昨天判定）
// 2) 比对 app.js 里 $("id") 引用的元素在 index.html 中都存在
// 运行：node scripts/test_date_nav.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const appSrc = fs.readFileSync(path.join(ROOT, "frontend", "app.js"), "utf8");
const html = fs.readFileSync(path.join(ROOT, "frontend", "index.html"), "utf8");

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

// ---------------------------------------------------------------- 函数抽取
// 截出「历史日期查看」到 resetHistDates 结束那一整段，里面全是纯函数
function extract(src, startMark, endMark) {
  const a = src.indexOf(startMark);
  const b = src.indexOf(endMark, a);
  if (a < 0 || b < 0) throw new Error("抽取失败: " + startMark);
  return src.slice(a, b);
}

const code = extract(
  appSrc,
  "// ---- 历史日期查看（活动 / 应用使用 / 媒体播放共用）----",
  "async function loadActivity() {"
);

// 桩：$ 与 viewData 只被 resetHistDates / updateDayNav 用到，这里不测它们
const $ = () => null;
const viewData = {};
const sandbox = { $, viewData, Date, Number, String, Object, isNaN, console };
const fn = new Function(
  "$",
  "viewData",
  code + "\n; return { todayStr, addDays, isToday, dateLabel, dayTitle, dayEmpty };"
);
const api = fn($, viewData);

// 固定「今天」为 2026-09-10：覆写 Date 后再取函数，保证断言不随运行日期漂移
const REAL_DATE = Date;
class FakeDate extends REAL_DATE {
  constructor(...args) {
    if (args.length === 0) super(2026, 8, 10, 15, 0, 0); // 2026-09-10 本地时间
    else super(...args);
  }
}
global.Date = FakeDate;
sandbox.Date = FakeDate;
const f = new Function(
  "$",
  "viewData",
  "Date",
  code + "\n; return { todayStr, addDays, isToday, dateLabel, dayTitle, dayEmpty };"
);
const T = f($, viewData, FakeDate);
global.Date = REAL_DATE;

console.log("== todayStr / isToday ==");
eq("todayStr 本地日期", T.todayStr(), "2026-09-10");
eq("isToday(今天)", T.isToday("2026-09-10"), true);
eq("isToday(昨天)", T.isToday("2026-09-09"), false);
eq("isToday(空值)", T.isToday(""), false);
eq("isToday(undefined)", T.isToday(undefined), false);

console.log("== addDays 边界 ==");
eq("今天 -1", T.addDays("2026-09-10", -1), "2026-09-09");
eq("跨月往前（10/1 -> 9/30）", T.addDays("2026-10-01", -1), "2026-09-30");
eq("跨月往后（9/30 -> 10/1）", T.addDays("2026-09-30", 1), "2026-10-01");
eq("跨年往前（2026/1/1 -> 2025/12/31）", T.addDays("2026-01-01", -1), "2025-12-31");
eq("跨年往后（2025/12/31 -> 2026/1/1）", T.addDays("2025-12-31", 1), "2026-01-01");
eq("闰年 2028/2/28 +1", T.addDays("2028-02-28", 1), "2028-02-29");
eq("平年 2026/2/28 +1", T.addDays("2026-02-28", 1), "2026-03-01");
eq("非法输入兜底为今天", T.addDays("不是日期", -1), "2026-09-10");
eq("+0 不变", T.addDays("2026-09-10", 0), "2026-09-10");

console.log("== dateLabel / dayTitle ==");
eq("今天", T.dateLabel("2026-09-10"), "今天");
eq("昨天", T.dateLabel("2026-09-09"), "昨天");
eq("前天", T.dateLabel("2026-09-08"), "9月8日");
eq("跨年日期带年份", T.dateLabel("2025-12-31"), "2025年12月31日");
eq("null 视为今天", T.dateLabel(null), "今天");
eq("标题-今天", T.dayTitle("2026-09-10", "活动明细"), "今日活动明细");
eq("标题-昨天", T.dayTitle("2026-09-09", "活动明细"), "昨日活动明细");
eq("标题-更早", T.dayTitle("2026-09-08", "活动明细"), "9月8日活动明细");

console.log("== dayEmpty 空态文案 ==");
eq("今天空态", T.dayEmpty("2026-09-10", "播放记录"), "今日暂无播放记录");
eq("历史空态", T.dayEmpty("2026-09-08", "播放记录"), "9月8日暂无播放记录");

console.log("== 元素 id 引用一致性 ==");
const htmlIds = new Set();
const idRe = /\bid="([^"]+)"/g;
let m;
while ((m = idRe.exec(html)) !== null) htmlIds.add(m[1]);

const used = new Set();
const useRe = /\$\("([^"]+)"\)/g;
while ((m = useRe.exec(appSrc)) !== null) used.add(m[1]);

const missing = [...used].filter((id) => !htmlIds.has(id));
if (missing.length) {
  fail++;
  console.log("  FAIL html 中缺少元素 id: " + missing.join(", "));
} else {
  pass++;
  console.log("  PASS app.js 引用的 " + used.size + " 个 id 在 index.html 中全部存在");
}

// 本次新增的导航元素必须齐全
const navIds = [
  "actPrevDay", "actNextDay", "actDayLabel", "actTitle",
  "appuPrevDay", "appuNextDay", "appuDayLabel", "appuTitle",
  "audioPrevDay", "audioNextDay", "audioDayLabel", "audioTitle",
];
const navMissing = navIds.filter((id) => !htmlIds.has(id));
if (navMissing.length) {
  fail++;
  console.log("  FAIL 缺少日期导航元素: " + navMissing.join(", "));
} else {
  pass++;
  console.log("  PASS 三个二级页的日期导航元素齐全（12 个）");
}

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
