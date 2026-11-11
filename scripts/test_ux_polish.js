// Run real keyboard and period-label handlers without changing user data.
const assert = require('assert');
const fs = require('fs');
const path = require('path');
const vm = require('vm');
const read = name => fs.readFileSync(path.join(__dirname, '../frontend/js', name), 'utf8').replace(/\r\n/g, '\n');
function pick(source, name) {
  const start = source.indexOf('function ' + name + '(');
  assert(start >= 0, name + ' exists');
  return source.slice(start, source.indexOf('\n}', start) + 2);
}
const els = new Map();
function $(id) {
  if (!els.has(id)) {
    const classes = new Set(['hidden']);
    els.set(id, {textContent:'', classList:{contains:c=>classes.has(c),
      add:c=>classes.add(c), remove:c=>classes.delete(c)}});
  }
  return els.get(id);
}
let closed=0, cancelled=0, confirms=0;
const ctx = {$, document:{querySelectorAll:()=>[]},
  closeWindow:()=>closed++, hideOtForm:()=>{cancelled++; $('otForm').classList.add('hidden');},
  hideConfirm: result=>{assert.equal(result,false); confirms++; $('confirmModal').classList.add('hidden');}};
vm.createContext(ctx);
vm.runInContext(pick(read('boot.js'), 'handleEscapeKey'), ctx);
function escape(extra={}) {ctx.handleEscapeKey({key:'Escape', preventDefault(){}, ...extra});}
$('otForm').classList.remove('hidden');
$('confirmModal').classList.remove('hidden');
escape(); assert.equal(confirms,1); assert.equal(cancelled,0); assert.equal(closed,0);
escape(); assert.equal(cancelled,1); assert.equal(closed,0);
escape(); assert.equal(closed,1);
$('restoreMask').classList.remove('hidden'); escape(); assert.equal(closed,1);
$('restoreMask').classList.add('hidden'); escape({isComposing:true}); assert.equal(closed,1);
ctx.handleEscapeKey({key:'Enter'}); assert.equal(closed,1);

vm.runInContext('let curBillSpan = "week";\n' + pick(read('bill.js'), 'setBillSpanUI'), ctx);
for (const [span, unit, period, window] of [
  ['week','周','这一周','最近八周'], ['month','月','这个月','最近八个月'], ['year','年','这一年','最近八年'],
  ['invalid','周','这一周','最近八周']]) {
  ctx.setBillSpanUI(span);
  assert.equal($('trendIncomeLabel').textContent, unit+'入账');
  assert.equal($('trendSlackLabel').textContent, unit+'摸鱼率');
  assert.equal($('heatEmptyText').textContent, period+'还没有键鼠记录');
  assert.equal($('bodyEmptyText').textContent, period+'还没有键鼠记录');
  assert.equal($('trendEmptyText').textContent, window+'还没有打工记录');
}
console.log('UX polish: Escape priority and week/month/year copy passed');
