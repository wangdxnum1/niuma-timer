// Real monitor loaders and date controls, with fixed time and deferred IPC.
const assert=require('assert'),fs=require('fs'),path=require('path'),vm=require('vm');
const source=fs.readFileSync(path.join(__dirname,'../frontend/js/monitor.js'),'utf8');
const html=fs.readFileSync(path.join(__dirname,'../frontend/index.html'),'utf8');
for(const key of ['act','appu','audio'])for(const suffix of ['PickDate','Today','LoadBox','LoadText','Retry'])assert(html.includes('id="'+key+suffix+'"'),'missing history control '+key+suffix);
const {rsRead}=require('./lib/rs_sources');
function fields(file,name){const body=rsRead(file).match(new RegExp('pub struct '+name+'\\s*\\{([\\s\\S]*?)\\n\\}'))[1];return [...body.matchAll(/pub (\w+):/g)].map(m=>m[1]).sort();}
const bucket=Object.fromEntries(fields('activity.rs','HourBucket').map(k=>[k,0]));
const fixtures={act:{date:'2026-11-11',hourly:[],totals:bucket,top_keys:[],active_hours:0,hook_ok:true},
  appu:{date:'2026-11-11',apps:[],categories:[],hourly:[],watch_ok:true},
  audio:{date:'2026-11-11',apps:[],hourly:[],watch_ok:true}};
for(const [key,file,name] of [['act','activity.rs','ActivitySummary'],['appu','app_usage.rs','AppUsageSummary'],['audio','audio_usage.rs','AudioUsageSummary']])assert.deepEqual(Object.keys(fixtures[key]).sort(),fields(file,name));
function element(){const classes=new Set(),events={},attrs={};return {value:'',disabled:false,textContent:'',innerHTML:'',style:{},children:[],
  classList:{add:c=>classes.add(c),remove:c=>classes.delete(c),contains:c=>classes.has(c),toggle(c,on){on?classes.add(c):classes.delete(c);}},
  appendChild(n){this.children.push(n);},append(...n){this.children.push(...n);},
  addEventListener(k,fn){events[k]=fn;},fire(k){return events[k]?.();},
  setAttribute(k,v){attrs[k]=v;},getAttribute:k=>attrs[k],querySelectorAll(){return [];}};}
const FIXED=new Date(2026,10,11,10).getTime();
let clockTime=FIXED;
class FakeDate extends Date{constructor(...args){super(...(args.length?args:[clockTime]));}static now(){return clockTime;}}
function harness(key,view){
  const els=new Map(),requests=[],toasts=[];const $=id=>{if(!els.has(id))els.set(id,element());return els.get(id);};
  const sections=[element(),element(),element()];$(view).querySelectorAll=()=>sections;
  const ctx={$,Date:FakeDate,curView:view,viewData:{activity:null,appu:null,audio:null},
    monitors:{activity:true,app_usage:true,audio:true},lastStatus:null,fmtDist:()=>'',fmtDurCN:()=>'',
    document:{createElement:element},flog(){},console:{error(){}},showToast:msg=>toasts.push(msg),
    invoke:(cmd,args)=>new Promise((resolve,reject)=>requests.push({cmd,args,resolve,reject}))};
  vm.createContext(ctx);vm.runInContext(source,ctx);
  vm.runInContext('renderChart=()=>{};renderTopKeys=()=>{};renderHourChart=()=>{};renderCategories=()=>{};renderAppRows=()=>{};renderSlackBurn=()=>{};',ctx);
  return {$,requests,ctx,sections,toasts,run:code=>vm.runInContext(code,ctx)};
}
async function main(){
  for(const [key,view,cache,load] of [['act','viewAct','activity','loadActivity'],['appu','viewApp','appu','loadAppUsage'],['audio','viewAudio','audio','loadAudioUsage']]){
    const {$,requests,ctx,sections,run}=harness(key,view);
    $(key+'PickDate').value='2024-02-29';const older=$(key+'PickDate').fire('change');
    assert.equal(requests.length,1,'date picker loads '+key);assert.equal(requests[0].args.date,'2024-02-29');
    assert(sections.every(s=>s.classList.contains('hidden')),'old details hidden during date change');
    await run(load+'()');assert.equal(requests.length,1,'same-date polling coalesced');
    $(key+'PickDate').value='2025-12-31';const newer=$(key+'PickDate').fire('change');
    requests[1].resolve({...fixtures[key],date:'2025-12-31'});await newer;
    requests[0].resolve({...fixtures[key],date:'2024-02-29'});await older;
    assert.equal(ctx.viewData[cache].date,'2025-12-31','stale success ignored');
    assert(sections.every(s=>!s.classList.contains('hidden')),'current details visible');
    ctx.document.activeElement=$(key+'PickDate');
    $(key+'PickDate').value='';run('updateDayNav("'+key+'")');
    assert.equal($(key+'PickDate').value,'','background refresh must preserve unfinished date input');
    for(const bad of ['2026-11-12','2025-02-29','']){$(key+'PickDate').value=bad;await $(key+'PickDate').fire('change');}
    assert.equal(requests.length,2,'future, invalid and empty dates rejected');
    assert.equal($(key+'PickDate').value,'2025-12-31');
    ctx.document.activeElement=null;
    $(key+'PickDate').value='2026-10-01';const failing=$(key+'PickDate').fire('change');
    requests[2].reject('db unavailable');await failing;
    assert.equal($(key+'Retry').classList.contains('hidden'),false,'failure offers retry');
    const retry=$(key+'Retry').fire('click');requests[3].resolve({...fixtures[key],date:'2026-10-01'});await retry;
    const today=$(key+'Today').fire('click');assert.equal(requests[4].args.date,null,'today follows rollover');
    requests[4].resolve(fixtures[key]);await today;
    assert.equal($(key+'Today').disabled,true);assert.equal(ctx.viewData[cache].date,'2026-11-11');
    const pending=run(load+'()');run('curView="viewMain";resetHistDates()');
    requests[5].resolve(fixtures[key]);await pending;
    ctx.curView=view;
    $(key+'PickDate').value='2026-10-02';const staleError=$(key+'PickDate').fire('change');
    $(key+'PickDate').value='2026-10-03';const successful=$(key+'PickDate').fire('change');
    requests[7].resolve({...fixtures[key],date:'2026-10-03'});await successful;
    requests[6].reject('old failure');await staleError;
    assert.equal($(key+'LoadBox').classList.contains('hidden'),true,'stale errors cannot replace current success');
    const mismatch=run(load+'()');requests[8].resolve({...fixtures[key],date:'2026-09-01'});await mismatch;
    assert.equal(ctx.viewData[cache].date,'2026-10-03','mismatched response not cached');
    assert.equal($(key+'Retry').classList.contains('hidden'),false);
    const late=run(load+'()');
    const monitorFlag={act:'activity',appu:'app_usage',audio:'audio'}[key];
    ctx.monitors[monitorFlag]=false;await run(load+'()');
    requests[9].resolve({...fixtures[key],date:'2026-10-03'});await late;
    assert.equal(ctx.viewData[cache],null,'late result cannot revive disabled monitoring');
    assert.match($(key+'LoadText').textContent,/已关闭/);
    ctx.monitors[monitorFlag]=true;
    const oldDay=run('selectHistDate("'+key+'","2026-11-11",'+load+')');
    clockTime=new Date(2026,10,12,10).getTime();
    const newDay=run(load+'()');
    requests[11].resolve({...fixtures[key],date:'2026-11-12'});await newDay;
    requests[10].reject('previous day timeout');await oldDay;
    assert.equal(ctx.viewData[cache].date,'2026-11-12','new day supersedes pending yesterday');
    assert.equal($(key+'PickDate').value,'2026-11-12');
    assert.equal($(key+'PickDate').getAttribute('max'),'2026-11-12');
    assert.equal($(key+'Today').disabled,true);
    clockTime=FIXED;
  }
  console.log('Monitor history: date controls, stale responses, coalescing and retries passed');
}
const deadline=setTimeout(()=>{console.error('Monitor history test timed out');process.exit(1);},5000);
main().catch(e=>{console.error(e);process.exitCode=1;}).finally(()=>clearTimeout(deadline));
