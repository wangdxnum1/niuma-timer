// Work-hour coverage states through real homepage, bill, image model and CSV functions.
const assert=require('assert'),fs=require('fs'),vm=require('vm');
const source=require('./lib/fe_sources').feSource();
function fn(name){const a=source.search(new RegExp('function '+name+'\\(')),b=source.indexOf('\n}',a);assert(a>=0&&b>a,name);return source.slice(a,b+2);}
function el(){const classes=new Set();return {textContent:'',value:'off',checked:true,children:[],classList:{add:c=>classes.add(c),remove:c=>classes.delete(c),contains:c=>classes.has(c),toggle(c,on){on?classes.add(c):classes.delete(c);}},append(...items){this.children.push(...items);}};}
const els=new Map(),$=id=>{if(!els.has(id))els.set(id,el());return els.get(id);};
const ctx={$,viewData:{appu:null},lastStatus:{is_workday:true,hourly_rate:60},
  fmtMoney:n=>n.toFixed(2),fmtSlackEquiv:()=>'',isToday:d=>d==='2026-11-12',slackQuip:()=> '有记录才评价',
  moneyConfigured:()=>true,fmtDateRange:()=>'',weekBillQuip:()=> '有记录才评价',curBillSpan:'week',
  billIsReady:()=>true,downloadCsv:(name,csv)=>{ctx.csv=csv;},billData:null};
vm.createContext(ctx);vm.runInContext(['trendSlackText','renderSlackBurn','slackScope','slackEmpty','buildReportModel','csvCell','csvRows','exportWeekBillCsv'].map(fn).join('\n'),ctx);
const raw={date:'2026-11-12',categories:[{key:'slack',seconds:36000}]};
function render(usage){ctx.viewData.appu={...raw,work_usage:usage};ctx.renderSlackBurn();}
render({front_seconds:120,slack_seconds:60,legacy_seconds:0});
assert.equal($('sbRate').textContent,'摸鱼率 50%');assert.equal($('sbAmt').textContent,'1.00');
assert.equal($('sbMain').classList.contains('hidden'),false);
render({front_seconds:120,slack_seconds:0,legacy_seconds:0});assert.equal($('sbAmt').textContent,'0.00');
render({front_seconds:0,slack_seconds:0,legacy_seconds:900});
assert($('sbMain').classList.contains('hidden'));assert.match($('sbScope').textContent,/旧版数据无法/);assert.equal($('sbQuip').textContent,'');
render({front_seconds:0,slack_seconds:0,legacy_seconds:0});assert.match($('sbScope').textContent,/暂无工作时段/);
render(null);assert.match($('sbScope').textContent,/暂不可用/);
render({front_seconds:120,slack_seconds:60,legacy_seconds:900});assert.match($('sbScope').textContent,/旧版数据未纳入/);
ctx.lastStatus.is_workday=false;ctx.renderSlackBurn();assert($('slackBurn').classList.contains('hidden'));
ctx.lastStatus.is_workday=true;ctx.viewData.appu.date='2026-11-11';ctx.renderSlackBurn();assert($('slackBurn').classList.contains('hidden'));
const old={front_seconds:0,slack_legacy_seconds:900,slack_cost:0,slack_rate:0,buckets:[{label:'周一',front_seconds:0,slack_seconds:0,slack_legacy_seconds:900}]};
let model=ctx.buildReportModel(old);assert.equal(model.slackCost,null);assert.equal(model.slackPct,null);assert.equal(model.quip,'');assert.match(model.slackEmpty,/旧版/);
ctx.billData=old;ctx.exportWeekBillCsv();assert.match(ctx.csv,/摸鱼成本\(元\),\r?\n/);assert.match(ctx.csv,/旧版数据未纳入/);assert.match(ctx.csv,/,900/);
model=ctx.buildReportModel({...old,front_seconds:120,slack_cost:1,slack_rate:0.5});assert.equal(model.slackPct,50);assert.match(model.slackScope,/旧版数据未纳入/);
assert.equal(ctx.slackEmpty({slack_legacy_seconds:0}),'暂无工作时段记录');
console.log('Work-hour UI: accurate, zero, legacy, mixed, unavailable and export coverage passed');

assert.match(ctx.trendSlackText({slack_rate:null,slack_legacy_seconds:100}),/旧版数据无法重算/);
assert.match(ctx.trendSlackText({slack_rate:0.5,slack_legacy_seconds:100}),/50%.*旧版数据未纳入/);
assert.match(ctx.trendSlackText({slack_rate:0,slack_legacy_seconds:0}),/0%.*仅统计工作时段/);
const rust=fs.readFileSync(require('path').join(__dirname,'../src-tauri/src/insights.rs'),'utf8');
assert.match(rust,/pub slack_legacy_seconds: i64/);assert.match(rust,/slack_legacy_seconds: pb.slack_legacy_seconds/);
