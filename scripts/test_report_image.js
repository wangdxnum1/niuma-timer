// 周报图片（v1.6.0）契约测试：
//  1) Rust：export_image 命令 + safe_download_path 共用清洗 + 注册链 + capabilities
//  2) 前端：模型折算（功能性断言——真实调用 buildReportModel）+ canvas 绘制 + 保存链路降级
//  3) 结构：按钮/绑定/命令名
//
// 运行：node scripts/test_report_image.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const read = (p) => fs.readFileSync(path.join(ROOT, p), "utf-8").replace(/\r\n/g, "\n");
const rsCore = read("src-tauri/src/cmds_core.rs");
const rsMain = read("src-tauri/src/main.rs");
const caps = read("src-tauri/capabilities/default.json");
const html = read("frontend/index.html");
const billSrc = read("frontend/js/bill.js");
const bootSrc = read("frontend/js/boot.js");
const css = read("frontend/styles.css");

let pass = 0, fail = 0;
const eq = (name, a, b) => {
  if (a === b) { pass++; } else { fail++; console.log("FAIL " + name + "  期望 " + JSON.stringify(b) + " 实际 " + JSON.stringify(a)); }
};
const ok = (name, cond) => eq(name, !!cond, true);
const has = (name, hay, needle) => eq(name, typeof hay === "string" && hay.includes(needle), true);

console.log("== Rust：export_image ==");
has("命令 export_image（base64 解码落盘）", rsCore, "pub(crate) fn export_image(filename: String, content_base64: String)");
has("与 export_csv 共用 safe_download_path 清洗", rsCore, "fn safe_download_path(filename: &str)");
ok("export_csv 改走共用清洗", /export_csv[\s\S]{0,600}let path = safe_download_path\(&filename\)/.test(rsCore));
has("报告图标题随跨度（写死周账单会让月跨度导出图穿帮）", billSrc, 'model.span === "month" ? "月账单" : "周账单"');
ok("空图片数据拒绝写入", rsCore.includes('"图片数据为空"'));
has("注册进 generate_handler", rsMain, "export_image,");
has("capabilities 自动补齐 allow-export-image", caps, "allow-export-image");

console.log("== 前端：模型折算（真实调用） ==");
// 从 bill.js 抽出 buildReportModel / spanQuipText 源码（到行首 "}" 为止），配桩真实调用
const m = billSrc.match(/function buildReportModel\(bill, leaveDays\) \{[\s\S]*?\n\}/);
const sq = billSrc.match(/function spanQuipText\(text, span\) \{[\s\S]*?\n\}/);
ok("buildReportModel 可提取", !!m);
ok("spanQuipText 可提取", !!sq);
if (m && sq) {
  const stubFmtDateRange = (s, e) => s + "~" + e;
  // 桩金句真实走 spanQuipText：验「本周/本月」措辞随跨度切换
  const spanQuipText = new Function("return " + sq[0])();
  const stubQuip = (pct, withMoney, span) =>
    spanQuipText("本周工资建议原路退回", span) + "@" + pct + "@" + (withMoney ? 1 : 0);
  const build = new Function(
    "fmtDateRange", "weekBillQuip", "moneyConfigured", "curBillSpan",
    m[0] + "\nreturn buildReportModel;"
  );
  const model = build(stubFmtDateRange, stubQuip, () => true, "week")({
    period_label: "第 37 周",
    period_start: "2026-09-07",
    period_end: "2026-09-13",
    total_income: 3000,
    base_salary: 2800,
    ot_fee: 200,
    ot_hours: 7.5,
    slack_cost: 123.45,
    slack_rate: 0.182,
    work_days: 5,
    work_hours: 47.5,
    buckets: [
      { label: "周一", salary: 400, is_workday: true, has_record: true },
      { label: "周六", salary: 0, is_workday: false, has_record: false },
    ],
    hardest: { date: "x", weekday: "周五", ot_hours: 3.5 },
    slackiest: null,
  });
  eq("期号与日期范围透传", model.label + "|" + model.range, "第 37 周|2026-09-07~2026-09-13");
  eq("进账 = total_income", model.income, 3000);
  eq("摸鱼率折成百分数", model.slackPct.toFixed(1), "18.2");
  eq("加班费与时长同行", model.ot + "|" + model.otHours, "200|7.5");
  eq("金句走带钱档位（周措辞）", model.quip, "本周工资建议原路退回@18.2@1");
  eq("模型带跨度（周）", model.span, "week");
  eq("桶条带工作日标记", model.bars[1].isWorkday, false);
  eq("极值缺失容错 null", model.slackiest, null);
}

console.log("== 月报图片（v1.7.0） ==");
has("spanQuipText 分档函数", billSrc, "function spanQuipText(text, span) {");
ok("金句按跨度替换措辞（本周→本月）", /span === "month" \? text\.replace\(\/本周\/g, "本月"\)/.test(billSrc));
ok("weekBillQuip 带 span 参数", /function weekBillQuip\(ratePct, withMoney, span\)/.test(billSrc));
ok("年跨度双重拦截（按钮禁用 + 函数早退）",
  /imgBtn\.disabled = curBillSpan === "year"/.test(billSrc) &&
  /if \(curBillSpan === "year"\) \{\s*\n\s*showToast\("年账单暂不支持存为图片", "err"\);/.test(billSrc));
ok("禁用态样式", css.includes(".ghost[disabled]"));
if (m && sq) {
  // 月跨度：同一模型，金句措辞「本周→本月」，span 字段随全局切换
  const stubFmtDateRange = (s, e) => s + "~" + e;
  const spanQuipText = new Function("return " + sq[0])();
  const stubQuip = (pct, withMoney, span) =>
    spanQuipText("本周工资建议原路退回", span) + "@" + pct + "@" + (withMoney ? 1 : 0);
  const mm = new Function(
    "fmtDateRange", "weekBillQuip", "moneyConfigured", "curBillSpan",
    m[0] + "\nreturn buildReportModel;"
  )(stubFmtDateRange, stubQuip, () => true, "month")({
    period_label: "2026 年 9 月", period_start: "2026-09-01", period_end: "2026-09-30",
    total_income: 1, slack_rate: 0.182, buckets: [],
  });
  eq("月跨度金句措辞切换", mm.quip, "本月工资建议原路退回@18.2@1");
  eq("模型带跨度（月）", mm.span, "month");
  // 月报休假天数：显式传入时透传，周/年调用（不传）为 null（drawReport 不渲染该行）
  eq("月跨度不传休假 → null", mm.leaveDays, null);
  const ml = new Function(
    "fmtDateRange", "weekBillQuip", "moneyConfigured", "curBillSpan",
    m[0] + "\nreturn buildReportModel;"
  )(stubFmtDateRange, stubQuip, () => true, "month")(
    { period_label: "2026 年 9 月", period_start: "2026-09-01", period_end: "2026-09-30", total_income: 1, slack_rate: 0.1, buckets: [] },
    3,
  );
  eq("月报休假天数透传", ml.leaveDays, 3);
}

console.log("== 前端：绘制与保存链路 ==");
ok("drawReport 2x 物理密度", /canvas\.width = 750 \* s/.test(billSrc) && /ctx\.scale\(s, s\)/.test(billSrc));
ok("四行明细齐全（基础/加班/摸鱼/摸鱼率）", /"基础工资"/.test(billSrc) && /"加班费"/.test(billSrc) &&
  /"摸鱼成本"/.test(billSrc) && /"摸鱼率"/.test(billSrc));
ok("休息日柱与工作日柱视觉区分", /rgba\(255,255,255,0\.22\)/.test(billSrc));
ok("金句入图", /ctx\.fillText\(model\.quip/.test(billSrc));
has("按钮 billImageBtn", html, 'id="billImageBtn"');
has("boot 绑定 saveBillImage", bootSrc, '$("billImageBtn").addEventListener("click", saveBillImage);');
has("命令名 export_image", billSrc, 'invoke("export_image", {');
ok("无账单数据先引导（不画空图）", /先看一眼本期账单/.test(billSrc));
ok("剪贴板失败降级为仅保存（不阻断）", /typeof ClipboardItem === "function"/.test(billSrc) &&
  /已复制剪贴板，并保存到 /.test(billSrc));
ok("图片按桶条数自适应柱宽", /const barW = Math\.min\(52, \(areaR - areaL - gap \* \(n - 1\)\) \/ n\);/.test(billSrc));

ok("连点守卫：exportingImg 旗标 + 按钮禁用（同 downloadCsv 的 exportingCsv）",
  /let exportingImg = false;/.test(billSrc) &&
  /if \(btn\) btn\.disabled = true;/.test(billSrc) &&
  /exportingImg = false;[\s\S]{0,40}if \(btn\) btn\.disabled = false;/.test(billSrc));

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
