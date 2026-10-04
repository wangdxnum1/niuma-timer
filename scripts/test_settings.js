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
const appSrc = require("./lib/fe_sources").feSource();

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
  remind_sedentary_minutes: "50",
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
global.readBillStyle = () => "receipt";
global.readBillSpan = () => "week";
global.readSalaryMode = () => "monthly"; // v1.6.0 计薪方式分段读取（沙盒固定月聘）

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

console.log("== 久坐阈值钳制 1–120 ==");
store.remind_sedentary_minutes = "1";
eq("阈值 1 原样保存", api.readCfg().remind_sedentary_minutes, 1);
store.remind_sedentary_minutes = "0";
eq("阈值 0 回退默认 50", api.readCfg().remind_sedentary_minutes, 50);
store.remind_sedentary_minutes = "999";
eq("阈值 999 钳到 120", api.readCfg().remind_sedentary_minutes, 120);
store.remind_sedentary_minutes = "50";

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
const htmlSrc = fs.readFileSync(path.join(ROOT, "frontend", "index.html"), "utf8");
console.log("== 设置页六卡与备份区结构（v1.4.0 补强） ==");
["薪资与作息", "加班", "守护", "数据监控", "外观", "系统与数据"].forEach((t) => {
  ok("设置卡标题存在：" + t, htmlSrc.indexOf("<h2>" + t + "</h2>") >= 0);
});
ok(
  "备份区三 id + 还原重启遮罩齐全（backupStatus 已删：结果只走 toast）",
  ["backupNowBtn", "backupList", "backupDirHint", "restoreMask"].every(
    (id) => htmlSrc.indexOf('id="' + id + '"') >= 0
  ) && htmlSrc.indexOf('id="backupStatus"') === -1 && appSrc.indexOf("backupStatus") === -1
);
ok("立即备份成功只弹 toast，不往页面写长路径", appSrc.indexOf('showToast("备份成功') >= 0);
ok(
  "还原/删除按钮用 row-btn 样式（删除带 danger 变体）",
  appSrc.indexOf('className = "row-btn"') >= 0 &&
    appSrc.indexOf('className = "row-btn danger"') >= 0 &&
    appSrc.indexOf('row-actions') >= 0
);
ok("renderBackups 空态文案为「还没有备份」", appSrc.indexOf("还没有备份") >= 0);
ok(
  "broken 条目标不可用且不再渲染还原按钮（只留删除）",
  appSrc.indexOf("不可用") >= 0 &&
    /if \(!b\.broken\) \{[\s\S]{0,220}dataset\.backup = b\.name/.test(appSrc)
);
ok(
  "还原二次确认同时说清「覆盖」与「自动备份」",
  appSrc.indexOf("覆盖当前全部数据") >= 0 && appSrc.indexOf("自动备份") >= 0
);
ok(
  "还原成功后盖不可取消遮罩（不等 tick）",
  /invoke\("restore_backup"[\s\S]{0,200}restoreMask[\s\S]{0,80}remove\("hidden"\)/.test(appSrc)
);
ok(
  "备份列表只在启动拉一次、不进 tick（无 setInterval 备份轮询）",
  !/setInterval\([^)]*loadBackups/.test(appSrc)
);

// ---- 备份区使用逻辑重排（2026-10-04）：摘要 + 折叠 + 手动删除 ----
ok(
  "摘要行说清份数/占用/轮转策略",
  appSrc.indexOf("自动轮转只保留最近 20 份") >= 0 && appSrc.indexOf("占用") >= 0
);
ok(
  "长列表默认折叠（最近 3 份）+ 展开全部/收起",
  appSrc.indexOf("BACKUP_PREVIEW = 3") >= 0 &&
    appSrc.indexOf("展开全部 ") >= 0 &&
    appSrc.indexOf("收起列表") >= 0
);
ok(
  "每行带删除按钮（含残缺包）+ 二次确认 + 不影响当前数据",
  appSrc.indexOf("data-backup-delete") >= 0 &&
    appSrc.indexOf("删除后无法恢复，不影响当前数据") >= 0
);
ok(
  "删除走 delete_backup 命令并在成功后刷新列表",
  /invoke\("delete_backup"[\s\S]{0,120}loadBackups/.test(appSrc)
);

console.log("== 计薪方式（v1.6.0 时薪模式） ==");
// 1) 结构：分段控件 + 时薪输入行（默认隐藏）+ 月聘行有 id 供显隐（markup 在 index.html，
//    复用 L143 已读入的 htmlSrc）
ok(
  "薪资卡有计薪方式分段（monthly/hourly 两项）",
  /id="salaryModeSeg"[\s\S]{0,200}data-salary-mode="monthly"[\s\S]{0,120}data-salary-mode="hourly"/.test(
    htmlSrc
  )
);
ok(
  "时薪输入行存在且默认隐藏",
  /id="hourlyWageRow" class="hidden"/.test(htmlSrc) && /id="hourly_wage" type="number"/.test(htmlSrc)
);
ok(
  "月聘行/工作日行有 id 供模式显隐",
  /id="monthlySalaryRow"/.test(htmlSrc) && /id="workdaysOverrideRow"/.test(htmlSrc)
);
// 2) load 回填：setSalaryModeUI + 空值显示空串
ok(
  "load 回填计薪方式并应用显隐",
  /setSalaryModeUI\(cfg\.salary_mode \|\| "monthly"\)/.test(appSrc)
);
ok(
  "load 回填时薪：0/缺省显示空串（避免把已配时薪抹成 0）",
  /hourly_wage"\)\.value = cfg\.hourly_wage \? cfg\.hourly_wage : ""/.test(appSrc)
);
// 3) readCfg 采集：salary_mode 必传，时薪留空不传（保旧值）
ok(
  "readCfg 采集 salary_mode 与 hourly_wage",
  /salary_mode: readSalaryMode\(\)/.test(appSrc) &&
    /wageRaw !== ""\) cfg\.hourly_wage = parseFloat\(wageRaw\) \|\| 0/.test(appSrc)
);
// 4) 绑定：时薪失焦存，分段点击即存
ok(
  "时薪输入失焦自动保存",
  /"hourly_wage",\s*\n\s*"am_start"/.test(appSrc)
);
ok(
  "计薪分段点击即存并应用显隐",
  /setSalaryModeUI\(b\.dataset\.salaryMode\);\s*\n\s*saveNow\(\)/.test(appSrc)
);
// 5) 金额守卫统一口径：时薪模式看时薪输入框
ok(
  "moneyConfigured 时薪分支（有效时薪 > 0）",
  /moneyConfigured\(\) \{[\s\S]{0,120}salaryMode === "hourly"[\s\S]{0,160}monthly_salary/.test(
    appSrc
  )
);

console.log("== 配置加载门闸（load 失败 → 拒绝自动保存）==");
// 1) load() 成功解锁、失败上闸并挂横幅
ok(
  "load 成功后 configLoaded = true",
  /lastSaved = JSON\.stringify\(readCfg\(\)\);[\s\S]{0,80}configLoaded = true;[\s\S]{0,60}showCfgLoadError\(false\)/.test(
    appSrc
  )
);
ok(
  "load 失败后 configLoaded = false 并显示横幅",
  /configLoaded = false;[\s\S]{0,60}showCfgLoadError\(true\)/.test(appSrc)
);
// 2) 横幅结构 + 重试按钮重新走 load()
ok(
  "设置页有加载失败横幅与重试按钮",
  htmlSrc.indexOf('id="cfgLoadError"') >= 0 && htmlSrc.indexOf('id="cfgRetryBtn"') >= 0
);
ok(
  "重试按钮重新调用 load()",
  /\$\("cfgRetryBtn"\)\.addEventListener\("click", async \(\) => \{ await load\(\); \}\)/.test(appSrc)
);
// 3) 行为：configLoaded=false 时 doSave 不发 save_config、不更新快照
const gateCode =
  "let configLoaded = false;\n" +
  pick(/async function doSave\(\{ silent = false \} = \{\}\) \{[\s\S]*?\n\}/, "doSave") +
  "\n; return { doSave, setLoaded: (v) => { configLoaded = v; } };";
const saveCalls = [];
const toasts = [];
const gateApi = new Function(
  "invoke", "showToast", "flog", "silentRefresh",
  "return (function(){\n" + gateCode + "\n})();"
)(
  (cmd) => { saveCalls.push(cmd); return Promise.resolve(); },
  (msg, type) => toasts.push(msg + "|" + type),
  () => {},
  () => {}
);
global.readCfg = () => ({ workdays_override: null }); // doSave 成功路径引用，沙盒给空配置
(async () => {
  await gateApi.doSave({});
  ok("未加载时 doSave 不发 save_config", saveCalls.length === 0);
  ok("未加载时给出阻止提示", toasts.some((t) => t.indexOf("阻止自动保存") >= 0));
  await gateApi.doSave({ silent: true });
  ok("未加载时静默保存同样拒绝", saveCalls.length === 0 && toasts.length === 1);
  gateApi.setLoaded(true);
  await gateApi.doSave({});
  ok("加载成功后 doSave 正常保存", saveCalls.length === 1 && saveCalls[0] === "save_config");
  summary();
})();

function summary() {
  console.log("");
  console.log(pass + " passed, " + fail + " failed");
  process.exit(fail ? 1 : 0);
}
