// 设置页保存契约：
// 1) 活动柱状图 max 与柱高用同一套事件合计（曾经 max 只加 moves+left+keys）
// 2) 月薪/发薪日留空不挡住其它字段保存（不传该键，后端合并保留旧值）
// 3) 自定义副标题失焦会写盘；离开设置页也会 flush
// 4) 上班天数覆盖带所属年月
//
// 运行：node scripts/test_settings.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const appSrc = fs.readFileSync(path.join(ROOT, "frontend", "app.js"), "utf8");

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
function ok(label, cond) {
  eq(label, !!cond, true);
}

function pick(re, name) {
  const m = appSrc.match(re);
  if (!m) throw new Error("提取失败: " + name);
  return m[0];
}

const code = [
  pick(/function currentYearMonth\(\) \{[\s\S]*?\n\}/, "currentYearMonth"),
  pick(/function bucketEvents\(b\) \{[\s\S]*?\n\}/, "bucketEvents"),
  pick(/function numOrNull\(v\) \{[\s\S]*?\n\}/, "numOrNull"),
  pick(/function readCfg\(\) \{[\s\S]*?\n\}/, "readCfg"),
].join("\n");

const store = {
  monthly_salary: "",
  payday: "",
  workdays_override: "18",
  am_start: "09:00",
  am_end: "12:00",
  pm_start: "13:00",
  pm_end: "18:00",
  duration_format: "hms",
  tray_hover_card: true,
  tagline_style: "custom",
  tagline_custom: "hello",
  overtime_enabled: false,
  overtime_start: "",
  overtime_rate: "20",
  overtime_meal_enabled: true,
  overtime_meal: "20",
  monitor_activity: true,
  monitor_app_usage: false,
  monitor_audio: true,
  weekend_overtime: false,
  weekend_ot_start: "",
  overtime_rate_weekend: "",
  overtime_rate_holiday: "",
  app_whitelist_enabled: false,
  retention_days: "0",
};

global.$ = (id) => ({
  get value() {
    return store[id] !== undefined ? store[id] : "";
  },
  checked: !!store[id],
});
global.readWhitelist = () => [];

const api = new Function(
  "return (function(){\n" + code + "\n; return { bucketEvents, readCfg, currentYearMonth };\n})();"
)();

console.log("== bucketEvents / 柱状图口径 ==");
eq("右键+滚轮计入合计", api.bucketEvents({ right: 100, wheel: 50 }), 150);
eq("空桶为 0", api.bucketEvents({}), 0);
ok(
  "renderChart 的 max 使用 bucketEvents",
  /const max = Math\.max\(1, \.\.\.hourly\.map\(\(b\) => bucketEvents\(b\)\)\)/.test(appSrc)
);
ok(
  "renderChart 不再用残缺的 moves\\+left\\+keys max",
  !/hourly\.map\(\(b\) => \(b\.moves \|\| 0\) \+ \(b\.left \|\| 0\) \+ \(b\.keys \|\| 0\)\)/.test(
    appSrc
  )
);

console.log("== readCfg 留空不传月薪/发薪日 ==");
const cfg = api.readCfg();
ok("空月薪不在载荷里", !Object.prototype.hasOwnProperty.call(cfg, "monthly_salary"));
ok("空发薪日不在载荷里", !Object.prototype.hasOwnProperty.call(cfg, "payday"));
ok("监控开关仍会保存", cfg.monitor_app_usage === false);
eq("覆盖天数", cfg.workdays_override, 18);
eq("覆盖所属月", cfg.workdays_override_for, api.currentYearMonth());

store.monthly_salary = "12000";
store.payday = "10";
store.workdays_override = "";
const cfg2 = api.readCfg();
eq("填了月薪会传", cfg2.monthly_salary, 12000);
eq("填了发薪日会传", cfg2.payday, 10);
eq("清空覆盖 -> null", cfg2.workdays_override, null);
eq("清空覆盖所属月 -> null", cfg2.workdays_override_for, null);

console.log("== 保存路径 ==");
ok(
  "doSave 不再因空月薪整次 return",
  !/if \(\$\("monthly_salary"\)\.value\.trim\(\) === "" \|\| \$\("payday"\)\.value\.trim\(\) === ""\)/.test(
    appSrc
  )
);
ok(
  "tagline_custom 失焦会 saveIfChanged",
  /\$\("tagline_custom"\)\.addEventListener\("blur", saveIfChanged\)/.test(appSrc)
);
ok(
  "离开设置页会 saveIfChanged",
  /curView === "viewSettings"[\s\S]{0,120}saveIfChanged/.test(appSrc)
);

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
