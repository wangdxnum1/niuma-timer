// Execute the real overtime form handlers with deferred IPC, without user data.
const assert = require('assert');
const fs = require('fs');
const path = require('path');
const vm = require('vm');
const src = fs.readFileSync(path.join(__dirname, '../frontend/js/overtime.js'), 'utf8');
const rust = require('./lib/rs_sources').rsRead('overtime.rs');
function fields(name) {
  const body = rust.match(new RegExp('pub struct ' + name + '\\s*\\{([\\s\\S]*?)\\n\\}'))[1];
  return [...body.matchAll(/pub (\w+):/g)].map(m => m[1]).sort();
}
const record = {date:'2026-08-19', lock_time:'20:00', ot_start:'18:00', raw_hours:2,
  cross_midnight:false, valid_hours:2, fee:40, meal:20, total:60, source:0};
assert.deepEqual(Object.keys(record).sort(), fields('OvertimeRecord'), 'record fixture matches Rust');
const preview = (valid_hours, fee) => ({...record, valid_hours, fee});
const FIXED_MS = new Date(2026, 10, 11, 10, 0, 0).getTime();
class FakeDate extends Date {
  constructor(...args) { super(...(args.length ? args : [FIXED_MS])); }
  static now() { return FIXED_MS; }
}
function harness() {
  const els = new Map(), requests = [];
  const $ = id => {
    if (!els.has(id)) {
      const handlers = {}, attributes = {};
      let value = '';
      els.set(id, { get value() { return value; }, set value(v) { value = String(v); },
        checked: false, disabled: false, textContent: '', focus() {},
        classList: { add() {}, remove() {} },
        setAttribute(name, value) { attributes[name] = value; },
        getAttribute(name) { return attributes[name]; },
        addEventListener(event, handler) { handlers[event] = handler; },
        fire(event) { return handlers[event]?.(); } });
    }
    return els.get(id);
  };
  const ctx = { $, Date: FakeDate, console, showToast() {},
    invoke: (cmd, args) => new Promise((resolve, reject) => requests.push({cmd, args, resolve, reject})) };
  vm.createContext(ctx);
  vm.runInContext(src, ctx);
  ctx.record = record;
  vm.runInContext('editOtRecord(record)', ctx);
  return {$, requests, run: code => vm.runInContext(code, ctx)};
}
const flush = () => new Promise(setImmediate);
async function busyFeedback() {
  const { $, requests, run } = harness();
  $('otf_lock').value = '21:30';
  const calculation = $('otf_lock').fire('input');
  assert.equal($('otfSave').textContent, '计算中…');
  requests[0].resolve(preview(3.5, 70)); await calculation;
  assert.equal($('otfSave').textContent, '更新');
  const saving = run('submitOtForm()');
  assert.equal($('otfSave').textContent, '保存中…');
  assert.equal($('otfSave').disabled, true);
  assert.equal($('otf_lock').disabled, true);
  assert.equal($('otfCancel').disabled, true);
  await run('submitOtForm()');
  assert.equal(requests.length, 2, 'duplicate save must not reach IPC');
  run('hideOtForm();openOtForm();editOtRecord(record)');
  assert.equal($('otfSave').textContent, '保存中…', 'pending save cannot be dismissed or replaced');
  requests[1].reject('disk full'); await saving;
  assert.equal($('otfSave').textContent, '更新');
  assert.equal($('otfSave').disabled, false);
  assert.equal($('otf_lock').disabled, false);
  assert.equal($('otfCancel').disabled, false);
  assert.match($('otfMsg').textContent, /disk full/);
  const retry = run('submitOtForm()');
  assert.equal(requests.length, 3, 'failure allows retry');
  requests[2].resolve({year:2026, month:8}); await retry;
  assert.equal($('otfSave').disabled, false);
}
async function hoursAndFeeLinkage() {
  const { $, requests, run } = harness();
  $('otf_lock').value = '21:30';
  const time = $('otf_lock').fire('input');
  requests[0].resolve(preview(3.5, 70)); await time;
  $('otf_hours').value = '2';
  const hours = $('otf_hours').fire('input');
  assert.equal(requests.length, 2, 'manual hours must recalculate the automatic fee');
  assert.equal(requests[1].args.input.valid_hours, 2);
  assert.equal($('otfSave').disabled, true);
  requests[1].resolve(preview(2, 40)); await hours;
  assert.equal($('otf_hours').value, '2');
  assert.equal($('otf_fee').value, '40', 'shown fee follows overridden hours');
  const saving = run('submitOtForm()');
  assert.equal(requests[2].args.input.valid_hours, 2);
  assert.equal(requests[2].args.input.fee, null);
  requests[2].resolve({year:2026, month:8}); await saving;

  run('editOtRecord(record)');
  $('otf_fee').value = '55'; $('otf_fee').fire('input');
  $('otf_hours').value = '1.5';
  const manual = $('otf_hours').fire('input');
  requests[3].resolve(preview(1.5, 30)); await manual;
  assert.equal($('otf_fee').value, '55', 'explicit fee survives hours edits');
  const manualSave = run('submitOtForm()');
  assert.equal(requests[4].args.input.valid_hours, 1.5);
  assert.equal(requests[4].args.input.fee, 55);
  requests[4].reject('test save failure'); await manualSave;

  $('otf_fee').value = ''; $('otf_fee').fire('input');
  $('otf_hours').value = '0'; const zero = $('otf_hours').fire('input');
  assert.equal(requests[5].args.input.valid_hours, 0);
  requests[5].resolve(preview(0, 0)); await zero;
  assert.equal($('otf_fee').value, '0', 'zero hours has zero automatic fee');
  const zeroSave = run('submitOtForm()');
  assert.equal(requests[6].args.input.valid_hours, 0);
  assert.equal(requests[6].args.input.fee, null);
  requests[6].reject('test save failure'); await zeroSave;

  $('otf_hours').value = ''; const cleared = $('otf_hours').fire('input');
  assert.equal(requests[7].args.input.valid_hours, null);
  requests[7].resolve(preview(3.5, 70)); await cleared;
  assert.equal($('otf_hours').value, '3.5', 'clearing hours restores automatic time calculation');
  assert.equal($('otf_fee').value, '70');
  const autoSave = run('submitOtForm()');
  assert.equal(requests[8].args.input.valid_hours, null);
  assert.equal(requests[8].args.input.fee, null);
  requests[8].reject('test save failure'); await autoSave;
}
async function main() {
  await busyFeedback();
  await hoursAndFeeLinkage();
  const h = harness(), {$, requests, run} = h;
  $('otf_lock').value = '21:30';
  const first = $('otf_lock').fire('input');
  assert.equal(requests.length, 1, 'changing leave time must request recalculation');
  assert.equal(requests[0].cmd, 'preview_overtime_record');
  assert.equal(requests[0].args.input.lock_time, '21:30');
  assert.equal(requests[0].args.input.valid_hours, null, 'old hours must not override calculation');
  assert.equal(requests[0].args.input.fee, null, 'old fee must not override calculation');
  assert.equal($('otfSave').disabled, true);
  await run('submitOtForm()');
  assert.equal(requests.length, 1, 'save must wait for recalculation');
  requests[0].resolve(preview(3.5, 70)); await first;
  assert.equal($('otf_hours').value, '3.5');
  assert.equal($('otf_fee').value, '70');
  assert.equal($('otf_meal').value, '20', 'preserve manual meal allowance');
  assert.equal($('otfSave').disabled, false);
  const saving = run('submitOtForm()');
  assert.equal(requests[1].cmd, 'save_overtime_record');
  assert.equal(requests[1].args.input.valid_hours, null);
  assert.equal(requests[1].args.input.fee, null);
  requests[1].reject('test save failure'); await saving;

  $('otf_lock').value = '22:00'; const older = $('otf_lock').fire('input');
  $('otf_lock').value = '23:00'; const newer = $('otf_lock').fire('input');
  requests[3].resolve(preview(5, 100)); await newer;
  requests[2].resolve(preview(4, 80)); await older;
  assert.equal($('otf_hours').value, '5', 'late response cannot overwrite newer time');
  assert.equal($('otf_fee').value, '100');

  $('otf_start').value = '19:00'; const changedStart = $('otf_start').fire('input');
  assert.equal(requests[4].args.input.ot_start, '19:00');
  requests[4].reject('invalid interval'); await changedStart;
  assert.equal($('otf_fee').value, '', 'failure cannot retain a stale fee');
  assert.match($('otfMsg').textContent, /计算失败/);
  $('otf_cross').checked = true; const crossed = $('otf_cross').fire('change');
  assert.equal(requests[5].args.input.cross_midnight, true);
  $('otf_hours').value = '0.5'; const hoursPending = $('otf_hours').fire('input');
  $('otf_fee').value = '15'; $('otf_fee').fire('input');
  requests[5].resolve(preview(8, 160)); await crossed;
  requests[6].resolve(preview(0.5, 10)); await hoursPending;
  assert.equal($('otf_hours').value, '0.5', 'manual input wins over pending preview');
  const manualSave = run('submitOtForm()');
  assert.equal(requests[7].args.input.valid_hours, 0.5);
  assert.equal(requests[7].args.input.fee, 15);
  requests[7].reject('test save failure'); await manualSave;

  const closed = $('otf_lock').fire('input');
  run('hideOtForm();editOtRecord(record)');
  requests[8].resolve(preview(9, 180)); await closed;
  assert.equal($('otf_hours').value, '2', 'closed form response cannot change reopened record');
  for (const request of requests) {
    assert.deepEqual(Object.keys(request.args.input).sort(), fields('ManualOvertimeInput'),
      'preview and save inputs match Rust');
  }
  await flush();
  console.log('Overtime form linkage: all passed');
}
const deadline = setTimeout(() => { console.error('Overtime test timed out'); process.exit(1); }, 5000);
main().catch(e => {console.error(e); process.exitCode = 1;}).finally(() => clearTimeout(deadline));
