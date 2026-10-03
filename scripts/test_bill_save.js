// Execute the real save functions and span click handler, without a live app.
const fs = require('fs');
const path = require('path');
const vm = require('vm');
const assert = require('assert');
const src = require('./lib/fe_sources').feSource();
function fn(name) {
  const start = src.search(new RegExp('(?:async )?function ' + name + '\\('));
  assert(start >= 0, name);
  return src.slice(start, src.indexOf('\n}', start) + 2);
}
(async () => {
  const cfg = {bill_span: 'week', workdays_override: null};
  const calls = [], toasts = [];
  let click, fail = false;
  const button = {dataset: {span: 'month'}, addEventListener: (_, cb) => {click = cb;}};
  const ctx = {
    lastSaved: JSON.stringify(cfg), lastOverride: null, weekOffset: 3,
    configLoaded: true, // 设置加载成功的常态；门闸行为（false 拒存）由 test_settings.js 钉
    readCfg: () => ({...cfg}),
    invoke: async (command, args) => {calls.push({command, args}); if (fail) throw Error('disk unavailable');},
    showToast: (...args) => toasts.push(args), silentRefresh() {}, loadBillTab() {},
    setBillSpanUI: span => {cfg.bill_span = span;},
    document: {querySelectorAll: () => [button]},
  };
  vm.createContext(ctx);
  vm.runInContext(['saveIfChanged', 'saveNow', 'doSave'].map(fn).join('\n'), ctx);
  const binding = src.lastIndexOf('document.querySelectorAll("#billSpanSeg .mon-seg-item").forEach');
  assert(binding >= 0);
  vm.runInContext(src.slice(binding, src.indexOf('\n});', binding) + 4), ctx);
  click(); await new Promise(setImmediate);
  assert.equal(calls[0].command, 'save_config');
  assert.equal(calls[0].args.cfg.bill_span, 'month');
  assert.equal(ctx.weekOffset, 0);
  assert.equal(toasts.length, 0, 'span success must be silent');
  click(); await new Promise(setImmediate);
  assert.equal(calls.length, 1, 'same span must not write again');
  fail = true; button.dataset.span = 'year';
  click(); await new Promise(setImmediate);
  assert.equal(toasts[0][1], 'err', 'save errors must remain visible');
  assert.equal(JSON.parse(ctx.lastSaved).bill_span, 'month', 'failure must remain retryable');
  fail = false; toasts.length = 0;
  ctx.saveNow(); await new Promise(setImmediate);
  assert.equal(toasts[0][0], '已自动保存', 'settings save must still show success');
  console.log('Bill save: silent success, persistence, deduplication, failure and settings feedback passed');
})().catch(e => {console.error(e); process.exit(1);});
