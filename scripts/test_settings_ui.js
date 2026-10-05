// Exercise the settings presentation models directly, without duplicating them.
const assert = require("assert");
const fs = require("fs");
const vm = require("vm");
const path = require("path");
const root = path.join(__dirname, "..");
const file = path.join(root, "frontend/js/settings_ui.js");
assert.ok(fs.existsSync(file), "settings presentation module must exist");
const context = vm.createContext({ console });
vm.runInContext(fs.readFileSync(path.join(root, "frontend/js/tl_math.js"), "utf8"), context);
vm.runInContext(fs.readFileSync(file, "utf8"), context);
const cfg = { salary_mode: "monthly", monthly_salary: "12000", hourly_wage: "50", payday: "10",
  am_start: "09:00", am_end: "12:00", pm_start: "13:00", pm_end: "18:00",
  overtime_enabled: true, overtime_rate: "20", overtime_meal_enabled: true, overtime_meal: "20",
  weekend_overtime: true, overtime_rate_weekend: "", overtime_rate_holiday: "",
  remind_sedentary_enabled: true, remind_sedentary_minutes: "50", focus_enabled: true,
  focus_min_minutes: "25", workdays_override: "", workdays_override_for: null };
let count = 0;
function test(name, fn) { fn(); count++; console.log("PASS " + name); }
const model = patch => context.settingsScheduleModel({ ...cfg, ...patch }, 18);
test("monthly schedule uses actual days and eight paid hours", () => {
  const m = model({}); assert.equal(m.valid, true); assert.equal(m.hours, 8);
  assert.equal(m.hourlyRate, 12000 / 18 / 8); assert.equal(m.dailyPay, 12000 / 18);
});
test("unknown workdays never become a guessed wage", () => {
  assert.equal(context.settingsScheduleModel(cfg, null).hourlyRate, null);
  assert.equal(context.settingsScheduleModel(cfg, 0).dailyPay, null);
});
test("hourly schedule does not need calendar days", () => {
  const m = context.settingsScheduleModel({ ...cfg, salary_mode: "hourly" }, null);
  assert.equal(m.hourlyRate, 50); assert.equal(m.dailyPay, 400);
});
test("inverted and overlapping work periods are invalid", () => {
  assert.equal(model({ am_end: "08:00" }).valid, false);
  assert.equal(model({ pm_start: "11:00" }).valid, false);
  assert.equal(model({ am_start: "", am_end: "" }).valid, false);
});
test("one empty-length work period remains valid", () => {
  assert.equal(model({ am_start: "12:00", am_end: "12:00" }).hours, 5);
  assert.equal(model({ am_start: "12:00", am_end: "12:00", pm_end: "13:00" }).valid, false);
});
test("workday overrides apply only to their own month", () => {
  assert.equal(context.settingsOverrideIsCurrent({workdays_override: 18, workdays_override_for: "2026-10"}, "2026-10"), true);
  assert.equal(context.settingsOverrideIsCurrent({workdays_override: 18, workdays_override_for: "2026-09"}, "2026-10"), false);
  assert.equal(context.settingsOverrideIsCurrent({workdays_override: null}, "2026-10"), false);
});
test("blank fees inherit and explicit zero remains zero", () => {
  assert.equal(context.settingsRateModel(cfg).holiday, 20);
  assert.equal(context.settingsRateModel({...cfg,overtime_rate_weekend: "0"}).holiday, 0);
  assert.equal(context.settingsRateModel({...cfg,overtime_rate_holiday: "0"}).holiday, 0);
});
test("range errors identify fields rather than silently clamp", () => {
  const errors = context.settingsValidationErrors({...cfg, payday: "32", focus_min_minutes: "9", overtime_rate: "-1"});
  assert.ok(errors.some(x=>x.id === "payday"));
  assert.ok(errors.some(x=>x.id === "focus_min_minutes"));
  assert.ok(errors.some(x=>x.id === "overtime_rate"));
});
test("disabled dependent values do not block unrelated settings", () => {
  const errors = context.settingsValidationErrors({...cfg, overtime_enabled: false, overtime_rate: "-1", focus_enabled: false, focus_min_minutes: "9"});
  assert.equal(errors.length, 0);
});
test("optional fees and retained core blanks are valid", () => {
  assert.equal(context.settingsValidationErrors({...cfg, monthly_salary: "", payday: ""}).length, 0);
  assert.equal(context.settingsValidationErrors({...cfg, overtime_rate_weekend: "0"}).length, 0);
});
test("manual monthly days require a count, hourly mode does not", () => {
  assert.ok(context.settingsValidationErrors({...cfg,workdays_manual:true}).some(x=>x.id === "workdays_override"));
  assert.equal(context.settingsValidationErrors({...cfg,workdays_manual:true,salary_mode:"hourly"}).length, 0);
});
test("malformed clock values cannot produce a pay preview", () => {
  assert.equal(model({am_start:"09:65"}).valid, false);
  assert.equal(model({am_start:"9:00oops"}).valid, false);
});
test("hidden native badInput never blocks an unrelated save", () => {
  const runtime = vm.createContext({console});
  vm.runInContext(fs.readFileSync(path.join(root,"frontend/js/tl_math.js"),"utf8"),runtime);
  vm.runInContext(fs.readFileSync(file,"utf8"),runtime);
  runtime.settingsFormValues = () => cfg;
  let errors;
  runtime.showSettingsFieldErrors = value => { errors=value; };
  const input = {id:"monthly_salary",validity:{badInput:true},matches:()=>false,closest:()=>({})};
  runtime.document = {querySelectorAll:()=>[input]};
  assert.equal(runtime.validateSettingsForm(),true);
  input.closest=()=>null;
  assert.equal(runtime.validateSettingsForm(),false);
  assert.equal(errors[0].id,"monthly_salary");
  runtime.settingsFormValues=()=>({...cfg,salary_mode:"hourly"});
  assert.equal(runtime.validateSettingsForm(),true);
});
test("disabled dependent drafts retain last saved values", () => {
  const snapshot=context.settingsPreserveInactiveValues({...cfg,overtime_enabled:false,overtime_rate:"-1"}, {overtime_rate:20});
  assert.equal(snapshot.overtime_rate,20);
  assert.equal(context.settingsPreserveInactiveValues({...cfg,overtime_enabled:false,overtime_rate:30}, {overtime_rate:20}).overtime_rate,30);
  assert.equal(context.settingsPreserveInactiveValues({...cfg,focus_enabled:false,focus_min_minutes:10}, {focus_min_minutes:25}, {...cfg,focus_enabled:false,focus_min_minutes:"-"}).focus_min_minutes,25);
  const hidden={...cfg,weekend_overtime:false,overtime_rate_weekend:null};
  const blank={...hidden,overtime_rate_weekend:""};
  assert.equal(context.settingsPreserveInactiveValues({...hidden},{overtime_rate_weekend:30},blank).overtime_rate_weekend,null);
  assert.equal(context.settingsPreserveInactiveValues({...hidden},{overtime_rate_weekend:30},{...blank,invalid_native:["overtime_rate_weekend"]}).overtime_rate_weekend,30);
});
test("manual days expire before a long-running app serializes a new month", () => {
  const runtime = vm.createContext({console});
  vm.runInContext(fs.readFileSync(file,"utf8"),runtime);
  let month="2026-10";
  const input={value:"18"}, fields={classList:{toggle(){}}};
  runtime.currentYearMonth=()=>month;
  runtime.$=id=>id==="workdays_override"?input:fields;
  runtime.document={querySelectorAll:()=>[]};
  runtime.setWorkdaysModeUI(true);
  month="2026-11";
  runtime.expireSettingsWorkdays();
  assert.equal(input.value,"");
  assert.equal(vm.runInContext("settingsManualDays",runtime),false);
});
test("success status fades after two seconds and new states cancel its timer", () => {
  const runtime = vm.createContext({console});
  vm.runInContext(fs.readFileSync(file,"utf8"),runtime);
  const classes=new Set(), timers=new Map();let nextTimer=0;
  const target={dataset:{},textContent:"",classList:{add:x=>classes.add(x),remove:x=>classes.delete(x)}};
  runtime.$=()=>target;
  runtime.setTimeout=(callback,delay)=>{const id=++nextTimer;timers.set(id,{callback:()=>{timers.delete(id);callback();},delay});return id;};
  runtime.clearTimeout=id=>timers.delete(id);
  runtime.setSettingsSaveState("ready");
  assert.equal(target.textContent,"","initial successful load should keep the header quiet");
  assert.equal(timers.size,0);
  runtime.setSettingsSaveState("saved");
  assert.equal(timers.size,1);
  const first=[...timers.values()][0];assert.equal(first.delay,2000);
  first.callback();assert.equal(classes.has("settings-status-faded"),true);
  runtime.setSettingsSaveState("saved");assert.equal(classes.size,0);
  runtime.setSettingsSaveState("saving");assert.equal(timers.size,0);
  assert.equal(target.textContent,"保存中…");assert.equal(classes.size,0);
  runtime.setSettingsSaveState("error");assert.equal(timers.size,0);
  assert.equal(target.textContent,"未保存，请重试");
  runtime.setSettingsSaveState("ready");assert.equal(target.textContent,"");
});
console.log(count + " settings model tests passed");

// Run the actual persistence functions with controllable IPC completion.
const saveSource = fs.readFileSync(path.join(root, "frontend/js/settings.js"), "utf8");
function extract(name) {
  const start = saveSource.search(new RegExp("(?:async )?function " + name + "\\("));
  return saveSource.slice(start, saveSource.indexOf("\n}", start) + 2);
}
function saveContext() {
  const cfg = { bill_span: "week", workdays_override: null };
  const calls = [], states = [], pending = [];
  const ctx = { cfg, calls, states, pending, console, configLoaded: true,
    lastSaved: JSON.stringify(cfg), lastOverride: null, settingsSaveQueue: Promise.resolve(), settingsSavePending: 0,
    readCfg: () => ({ ...cfg }), validateSettingsForm: () => true,
    setSettingsSaveState: state => states.push(state), showToast() {}, flog() {}, silentRefresh() {}, refreshSettingsUI() {},
    invoke: (cmd, args) => { calls.push(args.cfg); return new Promise((resolve,reject)=>pending.push({resolve,reject})); } };
  vm.createContext(ctx);vm.runInContext(["doSave", "saveIfChanged"].map(extract).join("\n"), ctx);
  return ctx;
}
(async () => {
  const turn = () => new Promise(setImmediate);
  const ctx = saveContext();ctx.cfg.bill_span = "month";
  const first = ctx.doSave();await turn();
  ctx.cfg.bill_span = "week";
  const reverted = ctx.saveIfChanged();await turn();
  assert.equal(ctx.calls.length, 1, "second write must wait for first");
  ctx.pending[0].resolve();await first;await turn();
  assert.equal(ctx.calls.length, 2, "reverting while save is pending must still persist final intent");
  assert.equal(ctx.calls[1].bill_span, "week");ctx.pending[1].resolve();await reverted;
  assert.equal(JSON.parse(ctx.lastSaved).bill_span,"week");
  const retry=saveContext();retry.cfg.bill_span="year";const failed=retry.doSave();await turn();
  retry.pending[0].reject(Error("disk unavailable"));assert.equal(await failed,false);
  assert.equal(JSON.parse(retry.lastSaved).bill_span,"week");
  const again=retry.doSave();await turn();retry.pending[1].resolve();assert.equal(await again,true);
  retry.configLoaded=false;assert.equal(await retry.doSave(),false);assert.equal(retry.calls.length,2);
  retry.configLoaded=true;retry.validateSettingsForm=()=>false;
  assert.equal(await retry.doSave(),false);assert.equal(retry.calls.length,2);
  console.log("Save runtime: serialized writes, pending revert, failure retry and guards passed");
  const heroSource=fs.readFileSync(path.join(root,"frontend/js/hero.js"),"utf8");
  const start=heroSource.indexOf("async function silentRefresh()");
  const refreshCode=heroSource.slice(start,heroSource.indexOf("\n}",start)+2);
  let resolveRefresh;
  const refreshCtx={lastSaved:"manual",settingsSavePending:0,accepted:[],flog(){},
    invoke:()=>new Promise(resolve=>{resolveRefresh=resolve;}),setSettingsWorkdays:n=>refreshCtx.accepted.push(n)};
  vm.createContext(refreshCtx);vm.runInContext(refreshCode,refreshCtx);
  const oldRefresh=refreshCtx.silentRefresh();refreshCtx.lastSaved="auto";resolveRefresh(18);await oldRefresh;
  assert.equal(refreshCtx.accepted.length,0,"old manual refresh cannot masquerade as automatic count");
  const currentRefresh=refreshCtx.silentRefresh();resolveRefresh(22);await currentRefresh;
  assert.equal(refreshCtx.accepted[0],22);
  console.log("Workday refresh runtime: stale responses ignored, current responses accepted");
})().catch(error=>{console.error(error);process.exitCode=1;});
