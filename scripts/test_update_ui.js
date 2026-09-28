const fs = require('fs');
const path = require('path');
const vm = require('vm');
const assert = require('assert');
const root = path.join(__dirname, '..');
const app = fs.readFileSync(path.join(root, 'frontend/app.js'), 'utf8');
const html = fs.readFileSync(path.join(root, 'frontend/index.html'), 'utf8');
const css = fs.readFileSync(path.join(root, 'frontend/styles.css'), 'utf8');
function fn(name) {
  const start = app.search(new RegExp('(?:async )?function ' + name + '\\('));
  assert(start >= 0, name);
  return app.slice(start, app.indexOf('\n}', start) + 2);
}
function element() {
  const classes = new Set();
  return {textContent: '', disabled: false, dataset: {}, scrollTop: 0,
    setAttribute(name, value) {this[name] = value;}, focus() {},
    classList: {toggle(c, on) {if(on) classes.add(c); else classes.delete(c);},
      add(c) {classes.add(c);}, remove(c) {classes.delete(c);}, contains(c) {return classes.has(c);}}};
}
(async () => {
  assert(html.includes('id="updBackBtn"'), 'explicit return button required');
  assert(!html.includes('id="updLaterBtn"'), 'ambiguous Later removed');
  assert(/\.upd-version-number\s*\{[^}]*font-size:\s*26px/s.test(css), 'readable version numbers');
  const els = new Map(); const $ = id => {if(!els.has(id)) els.set(id, element()); return els.get(id);};
  const pending = [];
  const ctx = {$, updateInfo:null, updateAnnounce:null, updateChecking:false, updateSettingsScroll:0,
    invoke: () => new Promise((resolve, reject) => pending.push({resolve, reject})),
    curView:'viewSettings', DETAIL_VIEWS:[], document:{querySelectorAll:()=>[]},
    saveIfChanged(){}, repaintCurrentView(){}, resetHistDates(){}};
  vm.createContext(ctx);
  vm.runInContext(['paintUpdate','loadUpdateInfo','showView','returnFromUpdate'].map(fn).join('\n'), ctx);
  $('viewSettings').scrollTop = 480;
  ctx.showView('viewUpdate');
  assert.equal(ctx.updateSettingsScroll,480);
  assert.equal($('updCheckBtn').disabled,true);
  assert.equal($('updCheckLabel').textContent,'检查中…');
  await ctx.loadUpdateInfo();
  assert.equal(pending.length,1,'duplicate checks must be ignored');
  pending[0].resolve({current:'1.4.0',latest:'1.5.0',has_update:true,installed:true});
  await new Promise(setImmediate);
  assert.equal($('updCurrent').textContent,'v1.4.0');
  assert.equal($('updLatest').textContent,'v1.5.0');
  assert($('updLatestCard').classList.contains('has-update'));
  assert.equal($('updCheckBtn').disabled,false);
  const retry = ctx.loadUpdateInfo(); pending[1].reject(Error('offline')); await retry;
  assert.equal($('updCheckLabel').textContent,'重新检查');
  assert.equal($('updCurrent').textContent,'v1.4.0','keep known current version on network failure');
  assert.equal($('updLatest').textContent,'—','do not misrepresent failed check as latest version');
  assert.equal($('updStatus').dataset.state,'error');
  $('viewSettings').scrollTop = 0;
  ctx.returnFromUpdate();
  assert.equal(ctx.curView,'viewSettings');
  assert.equal($('viewSettings').scrollTop,480);
  console.log('Update UI: navigation, scroll, versions, busy guard and retry passed');
})().catch(e=>{console.error(e);process.exit(1);});
