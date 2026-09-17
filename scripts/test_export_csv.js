// 导出 CSV 契约（加班明细 / 周账单）：
// 1) index.html：两个视图各有一个导出按钮 id
// 2) styles.css：.export-bar 工具条
// 3) app.js：csvCell / csvRows / downloadCsv / exportOvertimeCsv / exportWeekBillCsv 真实函数存在
// 4) 功能：用桩 downloadCsv 跑 exportOvertimeCsv / exportWeekBillCsv，断言 CSV 行/排序/转义/汇总正确
// 运行：node scripts/test_export_csv.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const appSrc = fs.readFileSync(path.join(ROOT, "frontend", "app.js"), "utf8");
const html = fs.readFileSync(path.join(ROOT, "frontend", "index.html"), "utf8");
const css = fs.readFileSync(path.join(ROOT, "frontend", "styles.css"), "utf8");

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

// 提取顶层 function 源码（括号配对，函数体不嵌套同名）
function extractFn(src, name) {
  const start = src.indexOf("function " + name + "(");
  if (start < 0) throw new Error("找不到函数 " + name);
  let depth = 0;
  let end = -1;
  for (let j = src.indexOf("{", start); j < src.length; j++) {
    if (src[j] === "{") depth++;
    else if (src[j] === "}") {
      depth--;
      if (depth === 0) {
        end = j;
        break;
      }
    }
  }
  return src.slice(start, end + 1);
}

// ---------------------------------------------------------------- 1. 结构
has("加班页导出按钮 otExportBtn", html, 'id="otExportBtn"');
has("账单页导出按钮 billExportBtn", html, 'id="billExportBtn"');
has("CSS 导出工具条 .export-bar", css, ".export-bar {");

// ---------------------------------------------------------------- 2. 函数存在
["csvCell", "csvRows", "downloadCsv", "exportOvertimeCsv", "exportWeekBillCsv"].forEach(
  (n) => has("app.js 含函数 " + n, appSrc, "function " + n + "(")
);

// ---------------------------------------------------------------- 3. 真实逻辑跑通
const sandbox = { lastOt: null, billData: null, captured: null };
// 桩 downloadCsv：捕获文件名与 CSV 文本（不触发真实 Blob 下载）
sandbox.downloadCsv = function (filename, csv) {
  sandbox.captured = { filename: filename, csv: csv };
};

// 抽取除 downloadCsv 外的四个函数（downloadCsv 由上面的桩提供），放进同一作用域
const fns = ["csvCell", "csvRows", "exportOvertimeCsv", "exportWeekBillCsv"]
  .map((n) => extractFn(appSrc, n))
  .join("\n");
const runner = new Function(
  "sandbox",
  "var lastOt = sandbox.lastOt;\n" +
    "var billData = sandbox.billData;\n" +
    "var downloadCsv = sandbox.downloadCsv;\n" +
    fns +
    "\n; sandbox.csvCell = csvCell;" +
    " sandbox.csvRows = csvRows;" +
    " sandbox.exportOvertimeCsv = exportOvertimeCsv;" +
    " sandbox.exportWeekBillCsv = exportWeekBillCsv;" +
    " sandbox._setLastOt = function (v) { lastOt = v; };" +
    " sandbox._setBillData = function (v) { billData = v; };"
);
runner(sandbox);

// csvCell 转义
eq("csvCell 普通文本", sandbox.csvCell("hello"), "hello");
eq("csvCell 含逗号包引号", sandbox.csvCell("a,b"), '"a,b"');
eq("csvCell 含引号翻倍", sandbox.csvCell('he"llo'), '"he""llo"');
eq("csvCell null 转空串", sandbox.csvCell(null), "");

// 加班导出：空数据不触发下载
sandbox._setLastOt({ records: [] });
sandbox.captured = null;
sandbox.exportOvertimeCsv();
eq("加班空数据不下载", sandbox.captured, null);

// 加班导出：正常数据 → 校验表头 / 升序 / 跨午夜 / 来源 / 汇总 / 文件名
sandbox._setLastOt({
  year: 2026,
  month: 9,
  total_all: 410,
  total_hours: 10.5,
  days: 10,
  records: [
    {
      date: "2026-09-03",
      cross_midnight: false,
      lock_time: "19:30",
      valid_hours: 1.5,
      fee: 60,
      meal: 0,
      total: 60,
      source: 0,
    },
    {
      date: "2026-09-01",
      cross_midnight: true,
      lock_time: "01:20",
      valid_hours: 2,
      fee: 80,
      meal: 30,
      total: 110,
      source: 1,
    },
  ],
});
sandbox.captured = null;
sandbox.exportOvertimeCsv();
const oc = sandbox.captured.csv;
const olines = oc.split("\r\n");
eq(
  "加班 CSV 首行表头",
  olines[0],
  "日期,是否跨午夜,锁屏/下班时刻,有效时长(小时),加班费(元),餐补(元),合计(元),来源"
);
eq("加班 CSV 按日期升序首条", olines[1], "2026-09-01,是,次日 01:20,2,80,30,110,手动");
eq("加班 CSV 次条", olines[2], "2026-09-03,否,19:30,1.5,60,0,60,自动");
eq("加班 CSV 文件名", sandbox.captured.filename, "加班明细_2026-09.csv");
eq("加班 CSV 含月度汇总段", oc.includes("（月度汇总）"), true);
eq("加班 CSV 汇总合计金额", oc.includes("合计金额(元),410"), true);
eq("加班 CSV 汇总总时长", oc.includes("总有效时长(小时),10.5"), true);

// 周账单导出
sandbox._setBillData({
  week_no: 37,
  week_start: "2026-09-07",
  week_end: "2026-09-13",
  is_current_week: false,
  total_income: 5200.5,
  base_salary: 4000,
  ot_fee: 1200,
  slack_cost: 0,
  work_hours: 42.5,
  work_days: 5,
  keys_total: 12345,
  clicks_total: 678,
  days: [
    {
      date: "2026-09-07",
      weekday: "一",
      is_workday: true,
      has_record: true,
      salary: 900,
      slack_seconds: 120,
    },
    {
      date: "2026-09-08",
      weekday: "二",
      is_workday: true,
      has_record: true,
      salary: 850,
      slack_seconds: 300,
    },
  ],
});
sandbox.captured = null;
sandbox.exportWeekBillCsv();
const bc = sandbox.captured.csv;
eq("周账单 CSV 含周汇总段", bc.includes("（周汇总）"), true);
eq("周账单 CSV 总进账", bc.includes("总进账(元),5200.5"), true);
eq(
  "周账单 CSV 每日明细表头",
  bc.includes("日期,星期,是否工作日,有记录,当日工资(元),摸鱼秒数"),
  true
);
eq("周账单 CSV 工作日行", bc.includes("2026-09-07,一,是,是,900,120"), true);
eq("周账单 CSV 文件名", sandbox.captured.filename, "周账单_2026-09-07.csv");

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
