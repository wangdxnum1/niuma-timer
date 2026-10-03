// Execute real rendering/load functions with isolated DOM and deferred IPC.
const fs = require('fs');
const vm = require('vm');
const assert = require('assert');
const src = require('./lib/fe_sources').feSource();
function fn(name) {
  const start = src.search(new RegExp('(?:async )?function ' + name + '\\('));
  const end = src.indexOf('\n}', start);
  assert(start >= 0 && end >= 0, name);
  return src.slice(start, end + 2);
}
function element() {
  return { textContent: '', children: [], style: {}, classList: {toggle(){},add(){},remove(){}},
    append(...children) { this.children.push(...children); }, closest() {return this;} };
}
(async () => {
  for (const [loader, data, paint, tab] of [
    ['loadWeekBill','billData','paintWeekBill','bill'],
    ['loadHourHeat','heatData','paintHourHeat','heat'],
    ['loadWeekTrend','trendData','paintWeekTrend','trend'],
    ['loadBodyBill','bodyData','paintBodyBill','body']
  ]) {
    const requests = [];
    const ctx = {curView:'viewBill',curBillTab:tab,curBillSpan:'year',weekOffset:0,billRequestGeneration:0,
      invoke:()=>new Promise(resolve=>requests.push(resolve)),flog(){},$:()=>element(),[paint](){}};
    vm.createContext(ctx); vm.runInContext(fn(loader),ctx);
    const older=ctx[loader]();ctx.curBillSpan='month'; const newer=ctx[loader]();
    requests[1]({span:'month'}); await newer;requests[0]({span:'year'}); await older;
    assert.equal(ctx[data].span,'month',loader+' must discard stale response');
  }
  const els = new Map();
  const ctx = {curView:'viewBill',bodyData:{record_days:20,clicks:200,keys:400,pixels:9600,wheel_ticks:40,
      days:[{date:'2026-01',weekday:'1 月',events:500}],week_start:'2026-01-01',week_end:'2026-12-31'},
    $:id=>{if(!els.has(id))els.set(id,element());return els.get(id);},document:{createElement:element},
    paintBillNav(){},fmtWan:String,todayStr:()=> '2026-09-26'};
  vm.createContext(ctx);vm.runInContext(fn('fmtDist')+'\n'+fn('paintBodyBill'),ctx);ctx.paintBodyBill();
  assert.equal(els.get('bodyClicksAvg').textContent,'日均 10 次');
  assert.equal(els.get('bodyBars').children[0].children[1].textContent,'1 月');
  console.log('Bill runtime: 4 stale-response cases, daily average and month label passed');
})().catch(e=>{console.error(e);process.exit(1);});
