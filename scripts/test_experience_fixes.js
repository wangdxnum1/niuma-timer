// Real frontend functions with isolated DOM and deferred IPC. No user data touched.
const fs = require('fs');
const path = require('path');
const vm = require('vm');
const assert = require('assert');
const ROOT = path.resolve(__dirname, '..');
const read = p => fs.readFileSync(path.join(ROOT, p), 'utf8').replace(/\r\n/g, '\n');
const sources = ['core','bill','insights'].map(n => read('frontend/js/'+n+'.js')).join('\n');
const allSources = require('./lib/fe_sources').feSource();
function fn(name, src=allSources) {
  const start = src.search(new RegExp('(?:async )?function '+name+'\\('));
  const end = src.indexOf('\n}', start);
  assert(start >= 0 && end >= 0, name);
  return src.slice(start,end+2);
}
function element() {
  const classes=new Set(), sub=new Map();
  let text='';
  return {children:[],style:{},value:'',checked:true,disabled:false,dataset:{},
    get textContent(){return text;},set textContent(v){text=v;this.children=[];},
    classList:{add:c=>classes.add(c),remove:c=>classes.delete(c),contains:c=>classes.has(c),
      toggle(c,on){if(on===undefined)on=!classes.has(c);on?classes.add(c):classes.delete(c);}},
    append(...items){this.children.push(...items);},appendChild(item){this.children.push(item);},
    querySelector(sel){if(!sub.has(sel))sub.set(sel,element());return sub.get(sel);},
    closest(){return this;},setAttribute(){},addEventListener(){}};
}
const monthly={period_label:'2026 年 9 月',period_start:'2026-09-01',period_end:'2026-09-30',
  total_income:9000,base_salary:8800,ot_fee:200,ot_hours:8,slack_cost:90,slack_rate:0.1,
  front_seconds:1000,work_days:20,work_hours:160,keys_total:100,clicks_total:30,
  is_current_period:false,prev_total:0,delta_pct:null,hardest:null,slackiest:null,
  buckets:[{date:'2026-09-01',label:'01',salary:440,is_workday:true,has_record:true,
    slack_seconds:100,ot_total:0,act_events:130,keys:100,clicks:30,slack_rate:0.1}]};
const yearly={...monthly,period_label:'2026 年',period_start:'2026-01-01',period_end:'2026-12-31',
  total_income:90000,is_current_period:true,buckets:[{...monthly.buckets[0],date:null,label:'9 月',salary:9000,is_workday:null,ot_total:200}]};
const zeroMilestones={ot_hours:0,ot_fee:0,keystrokes:0,distance_px:0,focus_minutes:0,active_days:0,first_date:null};
// Fixture fields must match the actual serialized Rust structures in both directions.
function assertFields(structName, fixture, file) {
  const source=read(file),start=source.indexOf('pub struct '+structName+' {');
  assert(start>=0,structName);const body=source.slice(start,source.indexOf('\n}',start));
  const fields=[...body.matchAll(/pub (\w+):/g)].map(m=>m[1]);
  assert.deepEqual(Object.keys(fixture).sort(),fields.sort(),structName+' fixture drift');
}
assertFields('PeriodBill',monthly,'src-tauri/src/weekbill.rs');
assertFields('BucketBill',monthly.buckets[0],'src-tauri/src/weekbill.rs');
assertFields('Milestones',zeroMilestones,'src-tauri/src/milestones.rs');
function harness() {
  const els=new Map(),exports=[],toasts=[];
  const get=id=>{if(!els.has(id))els.set(id,element());return els.get(id);};
  get('monthly_salary').value='10000';get('slack_equiv_unit').value='off';
  class FakeDate extends Date {constructor(...args){super(...(args.length?args:['2026-10-09T10:00:00']));}}
  const ctx={Date:FakeDate,$:get,showToast:(...args)=>toasts.push(args),todayStr:()=> '2026-10-09',
    document:{getElementById:get,createElement:element,querySelector:()=>null,querySelectorAll:()=>[]},
    window:{__TAURI__:{core:{invoke:(...args)=>ctx.invoke(...args)}},addEventListener(){}},
    invoke:async cmd=>cmd==='get_milestones'?zeroMilestones:cmd==='get_day_overrides'?[]:monthly};
  vm.createContext(ctx);vm.runInContext(sources,ctx);
  const run=code=>vm.runInContext(code,ctx);
  run('curView="viewBill"; curBillTab="report"; curBillSpan="month";');
  ctx.capture=(model,name)=>exports.push({model,name});
  run('drawReport = model => model; saveCanvasPng = async (model,name) => capture(model,name);');
  return {ctx,run,get,exports,toasts};
}
const flush=()=>new Promise(setImmediate);
const emptyBill={...monthly,total_income:0,base_salary:0,ot_fee:0,ot_hours:0,slack_cost:0,
  front_seconds:0,slack_rate:0,work_days:0,work_hours:0,keys_total:0,clicks_total:0,buckets:[]};
const cases={};
cases.stale=async()=>{
  const {ctx,run,get,exports}=harness();
  await run('loadMonthlyReport()');
  const requests=[];
  ctx.invoke=(cmd,args)=>cmd==='get_milestones'?Promise.resolve(zeroMilestones):
    new Promise((resolve,reject)=>requests.push({cmd,args,resolve,reject}));
  run('setBillSpanUI("year");setBillTabUI("report");');
  const pending=run('loadMonthlyReport()');
  assert.equal(get('reportImageBtn').disabled,true,'loading report must disable export');
  assert.equal(get('reportBody').classList.contains('hidden'),true,'old body must be hidden during a period change');
  await run('saveReportImage()');assert.equal(exports.length,0,'old monthly data must not export under year span');
  requests[0].resolve(yearly);await flush();
  assert.equal(get('reportBody').classList.contains('hidden'),true,'body waits for leave data');
  assert(!get('billWeekLabel').textContent.includes('2026 年 ·'),'nav must wait for the same complete snapshot');
  requests[1].resolve([]);await pending;
  assert.equal(get('reportTitle').textContent,'2026 年战绩');
  assert.equal(get('reportImageBtn').disabled,false);
  await run('saveReportImage()');assert.equal(exports[0].model.income,90000);
  assert.equal(exports[0].model.span,'year');
  const failing=run('loadMonthlyReport()');requests[2].reject(Error('disk unavailable'));await failing;
  assert.equal(get('reportBody').classList.contains('hidden'),true);
  assert.equal(get('reportRetryBtn').classList.contains('hidden'),false,'failed report needs a visible retry');
  await run('saveReportImage()');assert.equal(exports.length,1,'failure must not export cached report');
  const retry=run('loadMonthlyReport()');requests[3].resolve(yearly);await flush();requests[4].resolve([]);await retry;
  assert.equal(get('reportImageBtn').disabled,false,'successful retry restores export');

  // Bill images and CSV use the same period-ready contract.
  const bill=harness();bill.run('curBillTab="bill";');await bill.run('loadWeekBill()');
  bill.ctx.invoke=()=>new Promise(()=>{});
  bill.run('setBillSpanUI("year");');bill.run('loadWeekBill()');
  await bill.run('saveBillImage()');assert.equal(bill.exports.length,0,'bill image must reject old period');
  bill.ctx.downloadCsv=()=>bill.exports.push('csv');
  vm.runInContext(fn('exportWeekBillCsv'),bill.ctx);
  bill.run('exportWeekBillCsv()');assert.equal(bill.exports.length,0,'CSV must reject old period');
};
cases.empty=async()=>{
  const noData=harness();
  noData.ctx.invoke=async cmd=>cmd==='get_milestones'?zeroMilestones:cmd==='get_day_overrides'?[]:emptyBill;
  await noData.run('loadMonthlyReport()');
  assert.equal(noData.get('reportBody').classList.contains('hidden'),true,'no evidence must produce an empty report');
  assert.equal(noData.get('reportImageBtn').disabled,true,'empty report cannot export');
  await noData.run('saveReportImage()');assert.equal(noData.exports.length,0);
  for (const [bill,leaves] of [[{...emptyBill,ot_hours:2,ot_fee:40,total_income:40},[]],
    [emptyBill,[{date:'2026-09-03',kind:'年假'}]]]) {
    const h=harness();h.ctx.invoke=async cmd=>cmd==='get_milestones'?zeroMilestones:cmd==='get_day_overrides'?leaves:bill;
    await h.run('loadMonthlyReport()');
    assert.equal(h.get('reportBody').classList.contains('hidden'),false,'overtime/leave evidence must remain visible');
    const slackRow=h.get('reportRows').children.find(row=>row.children[0].textContent==='摸鱼率');
    assert.equal(slackRow.children[1].textContent,'暂无记录');
    assert.equal(h.get('reportQuote').classList.contains('hidden'),true);
    await h.run('saveReportImage()');
    assert.equal(h.exports[0].model.slackPct,null,'unknown rate must survive the export boundary');
    assert.equal(h.exports[0].model.slackCost,null);
    assert.equal(h.exports[0].model.quip,'','no evidence must not generate a judgment');
  }
  const known=harness();known.ctx.fixture={...monthly,slack_rate:0,slack_cost:0};
  const model=known.run('buildReportModel(fixture,0,"")');
  assert.equal(model.slackPct,0,'observed zero remains zero');
  assert(model.quip.length>0,'observed activity may have a quip');
};
cases.navigation=async()=>{
  const h=harness();h.run('curBillSpan="year";weekOffset=2;setBillTabUI("timeline");');
  assert.equal(h.get('billPeriodControls').classList.contains('hidden'),true,'timeline must hide unrelated period controls');
  const calls=[];h.ctx.invoke=(cmd,args)=>{calls.push({cmd,args});return new Promise(()=>{});};
  h.run('shiftWeek(1)');
  assert.equal(h.run('weekOffset'),2,'timeline must not change the remembered period');
  assert.equal(calls.length,0,'hidden global navigation must not reload an unrelated day');
  h.run('tlShift(1)');assert.equal(calls[0].cmd,'get_day_timeline');assert.equal(calls[0].args.offset,1);
  h.run('setBillTabUI("report")');
  assert.equal(h.get('billPeriodControls').classList.contains('hidden'),false,'period controls restore on report');
  assert.equal(h.get('billPgName').textContent,'年报');
  assert.equal(h.run('curBillSpan'), 'year');assert.equal(h.run('weekOffset'),2);
  h.run('setBillTabUI("bill");shiftWeek(1)');
  assert.equal(h.run('weekOffset'),3);assert.equal(calls[1].cmd,'get_bill');
  assert.equal(calls[1].args.span,'year');assert.equal(calls[1].args.offset,3);
};
cases.pause=async()=>{
  const h=harness();vm.runInContext(['fmtShortH','renderBadge','updateLeaveBtn','dynamicTagline'].map(n=>fn(n)).join('\n'),h.ctx);
  h.ctx.minutesOf=v=>{const [hours,minutes]=v.split(':').map(Number);return hours*60+minutes;};
  h.get('am_end').value='12:00';h.get('pm_start').value='13:30';
  for(const status of [
    {paused:true,is_workday:true,off_work:false,worked_h:1,to_off_h:8,day_off_kind:null},
    {paused:true,is_workday:false,off_work:false,worked_h:0,to_off_h:0,day_off_kind:'年假'},
    {paused:true,is_workday:false,off_work:false,worked_h:0,to_off_h:0,to_off_str:'今天休息'},
  ]) {
    h.ctx.status=status;h.run('renderBadge(status)');
    assert(h.get('statusBadge').textContent.includes('监控'),'paused badge must identify the paused scope');
    assert(!h.get('statusBadge').textContent.includes('钱先冻结'));
    const text=h.run('dynamicTagline(status)');
    assert(text.includes('监控')&&text.includes('暂停'),'pause has precedence in the dynamic tagline');
    assert(!text.includes('钱一直在涨'),'pause must not use the active-work tagline');
  }
  // Exercise the standalone hover renderer, retaining its pause branch.
  const hover=read('frontend/hover_card.html');
  const start=hover.indexOf('function render(s) {'),end=hover.indexOf('\n      }',start);
  assert(start>=0&&end>=0);
  const hctx={card:element(),animating:false,cfg:null,curEarned:0,document:{getElementById:h.get},
    fmtMoney:h.ctx.fmtMoney,fmtDur:()=>'',renderTimeline(){},Date:h.ctx.Date};
  vm.createContext(hctx);vm.runInContext(hover.slice(start,end+8),hctx);
  hctx.render({paused:true,is_workday:true,off_work:false,earned:10,hourly_rate:20,daily_hours:8,worked_h:1,to_off_h:7});
  assert(h.get('footText').textContent.includes('监控'));assert(!h.get('footText').textContent.includes('钱先冻结'));
};
cases.chart=async()=>{
  for(const [span,n,title] of [['week',7,'每日进账'],['year',12,'每月进账'],['month',28,'每日进账'],['month',31,'每日进账']]) {
    const h=harness(),draws=[];
    const drawing={scale(){},fillRect(x,y,w,height){draws.push({kind:'rect',x,y,w,height});},
      fillText(text,x,y){draws.push({kind:'text',text,x,y});},measureText:t=>({width:String(t).length*10}),
      createLinearGradient:()=>({addColorStop(){}})};
    h.ctx.document.createElement=()=>({getContext:()=>drawing});
    vm.runInContext(fn('drawReport'),h.ctx);
    h.ctx.fixture={...monthly,buckets:Array.from({length:n},(_,i)=>({...monthly.buckets[0],
      label:String(i+1),salary:1000,is_workday:span==='year'?null:true,date:span==='year'?null:monthly.buckets[0].date}))};
    h.run('curBillSpan='+JSON.stringify(span));
    h.run('drawReport(buildReportModel(fixture,0,""),1)');
    const bars=draws.filter(d=>d.kind==='rect'&&d.y===590&&d.height===170);
    assert.equal(bars.length,n);
    for(let i=0;i<n;i++) {
      assert(Number.isFinite(bars[i].x)&&bars[i].w>0);
      assert(bars[i].x>=55&&bars[i].x+bars[i].w<=695.00001,'bar stays inside chart');
      if(i)assert(bars[i].x-(bars[i-1].x+bars[i-1].w)>1,'bars need a visible gap');
    }
    assert(bars.at(-1).x+bars.at(-1).w>650,'chart must occupy the available width');
    assert.equal(draws.find(d=>d.kind==='text'&&d.y===545).text,title,'heading must match bucket granularity');
  }
  // Unknown rates must be drawable, rather than throwing on null.toFixed.
  const h=harness(),texts=[];
  h.ctx.document.createElement=()=>({getContext:()=>({scale(){},fillRect(){},fillText:t=>texts.push(t),
    measureText:t=>({width:String(t).length*10}),createLinearGradient:()=>({addColorStop(){}})})});
  vm.runInContext(fn('drawReport'),h.ctx);h.ctx.fixture={...emptyBill,ot_hours:2,ot_fee:40,total_income:40};
  h.run('drawReport(buildReportModel(fixture,0,""),1)');
  assert(texts.includes('暂无记录'));assert(!texts.some(t=>String(t).includes('天选牛马')));
};
(async()=>{
  const selected=process.argv[2]?[[process.argv[2],cases[process.argv[2]]]]:Object.entries(cases);
  let failures=0;
  for(const [name,test] of selected){try{assert(test,'unknown case '+name);await test();console.log('PASS '+name);}
    catch(e){failures++;console.error('FAIL '+name+': '+e.stack);}}
  console.log(selected.length+' experience cases, '+failures+' failed');process.exitCode=failures?1:0;
})();
