// Exercise the real frontend handshake without a WebView dependency.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const html = fs.readFileSync(path.join(__dirname, '../frontend/hover_card.html'), 'utf8');
const start = html.indexOf('      function listen(name, fn)');
assert(start >= 0);
const code = html.slice(start, html.indexOf('</script>', start));

async function run(failListener) {
  const pending = [], emitted = [], errors = [], handlers = {};
  let shows = 0, hides = 0, payload;
  vm.runInNewContext(code, {
    window: {__TAURI_INTERNALS__: {
      transformCallback: fn => fn,
      invoke(cmd, args) {
        if (cmd === 'plugin:event|listen') {
          handlers[args.event] = args.handler;
          return new Promise((resolve, reject) => pending.push({resolve, reject}));
        }
        assert.equal(cmd, 'plugin:event|emit');
        emitted.push(args.event);
        return Promise.resolve();
      },
    }},
    showCard() { shows++; }, hideCard() { hides++; }, render(data) { payload = data; },
    loadCfg() {}, refreshOt() {}, reportErr(e) { errors.push(e); },
  });
  assert.equal(shows, 0, 'page load must not independently show a tooltip');
  assert.equal(pending.length, 3);
  pending[0].resolve(); pending[1].resolve();
  await new Promise(setImmediate);
  assert.deepEqual(emitted, [], 'ready must wait for ALL listeners');
  if (failListener) pending[2].reject(Error('listener failed'));
  else pending[2].resolve();
  await new Promise(setImmediate);
  if (failListener) {
    assert.deepEqual(emitted, []);
    assert.equal(errors.length, 1);
  } else {
    assert.deepEqual(emitted, ['hover_ready']);
    handlers.hover_show(); handlers.hover_hide(); handlers.hover_show();
    handlers.hover_data({payload: {earned: 123}});
    assert.equal(shows, 2); assert.equal(hides, 1);
    assert.equal(payload.earned, 123);
    assert.equal(errors.length, 0);
  }
}

(async () => {
  assert(!html.includes('plugin:window|hide'), 'old frontend transitions must not hide a new hover');
  assert(!html.includes('visibilitychange'), 'Rust owns tooltip visibility');
  await run(false); await run(true);
  console.log('Hover handshake: listener readiness, failure, reentry and single visibility owner passed');
})().catch(e => { console.error(e); process.exit(1); });
