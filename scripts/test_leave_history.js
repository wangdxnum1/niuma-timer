// Real leave UI handlers with a fixed clock, deferred IPC and isolated DOM.
const assert = require('assert'), fs = require('fs'), path = require('path'), vm = require('vm');
const read = p => fs.readFileSync(path.join(__dirname, '..', p), 'utf8').replace(/\r\n/g, '\n');
const entrySource = require('./lib/rs_sources').rsRead('cmds_core.rs');
const entryBody = entrySource.match(/pub\(crate\) struct DayOverrideEntry\s*\{([\s\S]*?)\n\}/)[1];
assert.deepEqual([...entryBody.matchAll(/pub (\w+):/g)].map(m=>m[1]).sort(), ['date','kind'], 'leave fixtures match Rust fields');
function element(tag='div') {
  const classes=new Set(), handlers={}, attrs={}; let text='', value='';
  return {tag,children:[],disabled:false,open:false,style:{},
    get value(){return value;},set value(v){value=String(v);},
    get textContent(){return text+this.children.map(c=>c.textContent||'').join('');},
    set textContent(v){text=String(v);this.children=[];},
    classList:{add:c=>classes.add(c),remove:c=>classes.delete(c),contains:c=>classes.has(c),
      toggle(c,on){on?classes.add(c):classes.delete(c);}},
    setAttribute(k,v){attrs[k]=v;},getAttribute(k){return attrs[k];},
    append(...nodes){this.children.push(...nodes);},appendChild(n){this.children.push(n);},
    addEventListener(k,fn){handlers[k]=fn;},fire(k){return handlers[k]?.();},
    querySelectorAll(){return this.children.flatMap(c=>[...(c.tag==='button'?[c]:[]),...c.querySelectorAll()]);}};
}
const FIXED_MS=new Date(2026,10,11,10).getTime();
class FakeDate extends Date {constructor(...args){super(...(args.length?args:[FIXED_MS]));}static now(){return FIXED_MS;}}
function harness() {
  const els=new Map(), requests=[], toasts=[];
  const $=id=>{if(!els.has(id))els.set(id,element());return els.get(id);};
  $('dayOffKind').value='年假';
  const ctx={$,Date:FakeDate,document:{createElement:element},flog(){},tick(){},
    showToast:msg=>toasts.push(msg),invoke:(cmd,args)=>new Promise((resolve,reject)=>requests.push({cmd,args,resolve,reject}))};
  vm.createContext(ctx);
  const source=read('frontend/js/settings.js');
  vm.runInContext(source.slice(0,source.indexOf('async function load()')),ctx);
  return {$,requests,toasts,run:code=>vm.runInContext(code,ctx)};
}
const flush=()=>new Promise(setImmediate);
async function main() {
  const {$,requests,run,toasts}=harness();
  const current=run('loadDayOffs()');
  assert.equal($('dayOffMonth').value,'2026-11');
  assert.equal(requests[0].args.start,'2026-11-01');
  assert.equal(requests[0].args.end,'2026-11-30');
  const previous=run('shiftDayOffMonth(-1)');
  requests[1].resolve([{date:'2026-10-02',kind:'年假'}]);await previous;
  requests[0].resolve([{date:'2026-11-02',kind:'病假'}]);await current;
  assert.match($('dayOffList').textContent,/2026-10-02/);
  assert(!$('dayOffList').textContent.includes('2026-11-02'),'late month response discarded');
  const leap=run('loadDayOffs("2024-02")');
  assert.equal(requests[2].args.end,'2024-02-29'); requests[2].resolve([]);await leap;
  const december=run('loadDayOffs("2025-12")');requests[3].resolve([]);await december;
  const january=run('shiftDayOffMonth(1)');
  assert.equal(requests[4].args.start,'2026-01-01');requests[4].resolve([]);await january;
  $('dayOffDate').value='2025-02-10';
  const saving=run('addDayOff()');await run('addDayOff()');
  assert.equal(requests.length,6,'duplicate leave write prevented');
  $('dayOffKind').value='病假';requests[5].resolve();await flush();
  assert.equal(requests[6].args.start,'2025-02-01','historical save selects its month');
  requests[6].resolve([{date:'2025-02-10',kind:'年假'}]);await saving;
  assert.match($('dayOffList').textContent,/2025-02-10/);
  assert(toasts.some(t=>t.includes('年假')),'feedback uses the saved kind');
  const failed=run('loadDayOffs()');requests[7].reject('offline');await failed;
  assert.match($('dayOffList').textContent,/读取失败/);
  const retry=$('dayOffList').querySelectorAll()[0].fire('click');
  requests[8].resolve([]);await retry;
  assert.equal($('dayOffList').getAttribute('aria-busy'),'false');
  const failedSave=run('addDayOff()');
  requests[9].reject('disk full');await failedSave;
  assert.equal($('dayOffAdd').disabled,false,'write failure restores controls');
  assert.equal($('dayOffDate').value,'2025-02-10','failure preserves entered date');
  const removing=run('removeDayOff("2025-02-10")');await run('removeDayOff("2025-02-10")');
  assert.equal(requests.length,11,'duplicate cancellation prevented');
  assert.equal(requests[10].args.kind,null);
  requests[10].resolve();await flush();
  assert.equal(requests[11].args.start,'2025-02-01','cancellation preserves selected month');
  requests[11].resolve([]);await removing;
  const beforeInvalid=requests.length;
  await run('loadDayOffs("invalid")');
  assert.equal(requests.length,beforeInvalid,'invalid month does not issue a query');
  console.log('Leave history: month bounds, stale responses, historical save and retry passed');
}
const deadline=setTimeout(()=>{console.error('Leave history test timed out');process.exit(1);},5000);
main().catch(e=>{console.error(e);process.exitCode=1;}).finally(()=>clearTimeout(deadline));
