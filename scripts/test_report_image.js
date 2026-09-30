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
ok("空图片数据拒绝写入", rsCore.includes('"图片数据为空"'));
has("注册进 generate_handler", rsMain, "export_image,");
has("capabilities 自动补齐 allow-export-image", caps, "allow-export-image");

console.log("== 前端：模型折算（真实调用） ==");
// 从 bill.js 抽出 buildReportModel 源码（到行首 "}"" 为止），配桩真实调用
const m = billSrc.match(/function buildReportModel\(bill\) \{[\s\S]*?\n\}/);
ok("buildReportModel 可提取", !!m);
if (m) {
  const stubFmtDateRange = (s, e) => s + "~" + e;
  const stubQuip = (pct, withMoney) => withMoney ? "quip-money@" + pct : "quip-plain@" + pct;
  const build = new Function(
    "fmtDateRange", "weekBillQuip", "moneyConfigured",
    m[0] + "\nreturn buildReportModel;"
  )(stubFmtDateRange, stubQuip, () => true);
  const model = build({
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
  eq("金句走带钱档位", model.quip, "quip-money@18.2");
  eq("桶条带工作日标记", model.bars[1].isWorkday, false);
  eq("极值缺失容错 null", model.slackiest, null);
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

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
