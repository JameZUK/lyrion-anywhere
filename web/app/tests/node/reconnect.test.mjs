// Reconnect policy tests (node --test).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  AuthWait,
  CLOSE,
  FlushWatch,
  Outbox,
  RateRequest,
  SEEK_GIVE_UP_MS,
  SeekTimer,
  backoffMs,
  closeAction,
  describeSeek,
  streamFailureAction,
} from '../../static/player/reconnect.js';
import { readFileSync } from 'node:fs';

test('close codes map to the right recovery', () => {
  // Security-relevant: never retry with credentials the gateway rejected, and never fight
  // another connection that legitimately took over this device.
  assert.equal(closeAction(CLOSE.POLICY), 'stop');
  assert.equal(closeAction(CLOSE.REPLACED), 'stop');
  // Session gone: start afresh.
  assert.equal(closeAction(CLOSE.NO_SESSION), 'new-session');
  assert.equal(closeAction(CLOSE.GOING_AWAY), 'new-session');
  // Network trouble: resume the same session.
  assert.equal(closeAction(CLOSE.ABNORMAL), 'resume');
  assert.equal(closeAction(CLOSE.TRY_AGAIN), 'resume');
  assert.equal(closeAction(1011), 'resume');
});

test('backoff grows, is capped, and is jittered', () => {
  const mid = () => 0.5;
  assert.equal(backoffMs(0, mid), 500);
  assert.equal(backoffMs(1, mid), 1000);
  assert.equal(backoffMs(3, mid), 4000);
  assert.equal(backoffMs(10, mid), 5000);
  assert.equal(backoffMs(1000, mid), 5000);
  assert.ok(backoffMs(2, () => 0) >= 1600 && backoffMs(2, () => 1) <= 2400);
});

test('stream failures: retry network trouble, fail on definitive answers', () => {
  assert.equal(streamFailureAction(0), 'retry');
  assert.equal(streamFailureAction(502), 'retry');
  assert.equal(streamFailureAction(503), 'retry');
  for (const s of [400, 401, 403, 404, 410, 416]) assert.equal(streamFailureAction(s), 'fail', `${s}`);
});

test('outbox is bounded and drops oldest first', () => {
  const o = new Outbox(3);
  for (const x of [1, 2, 3, 4, 5]) o.push(x);
  assert.equal(o.length, 3);
  assert.equal(o.dropped, 2);
  assert.deepEqual(o.drain(), [3, 4, 5]);
  assert.equal(o.length, 0);
});

// Regression: the worker asked for a new AudioContext on every 20 ms tick until the page
// confirmed it, and the page built one per request (an AudioContext storm).
test('a rate change is requested once, not on every tick', () => {
  const r = new RateRequest();
  let ready = 44100;
  const asks = [];
  const tick = (rate) => r.want(rate, ready) && asks.push(rate);
  for (let i = 0; i < 50; i++) tick(44100); // already running at this rate
  assert.deepEqual(asks, []);
  for (let i = 0; i < 50; i++) tick(96000); // 50 ticks before the page is ready
  assert.deepEqual(asks, [96000]);
  ready = 96000;
  for (let i = 0; i < 10; i++) tick(96000);
  for (let i = 0; i < 10; i++) tick(48000);
  assert.deepEqual(asks, [96000, 48000]);
  // Back to a rate asked for before, after another one: asked again.
  ready = 48000;
  tick(96000);
  assert.deepEqual(asks, [96000, 48000, 96000]);
  assert.equal(r.want(0, ready), false);
  r.reset();
  assert.equal(r.want(44100, ready), true);
});

// Regression: after an unauthorised close, the worker reconnected only on a token sent with
// reconnect:true, so a renewal retried after being offline (a plain token update) never
// brought the player back.
test('waiting for a new token: any token reconnects, once, and only while waiting', () => {
  const w = new AuthWait();
  assert.equal(w.token(false), false); // not waiting: a routine token update
  w.refused();
  assert.equal(w.token(true), false); // somehow connected already
  assert.equal(w.token(false), true);
  assert.equal(w.token(false), false); // only once
  w.refused();
  assert.equal(w.token(false), true);
});

test('the worker goes through both guards', () => {
  const src = readFileSync(new URL('../../static/player/player-worker.js', import.meta.url), 'utf8');
  // Every rate request is gated by RateRequest.
  const posts = src.match(/postMessage\(\{ rate \}\)/g) ?? [];
  assert.equal(posts.length, 1);
  assert.match(src, /if \(rateRequest\.want\(rate, readyRate\)\) postMessage\(\{ rate \}\)/);
  // A token reconnects through AuthWait, which the unauthorised close arms.
  assert.match(src, /if \(authWait\.token\(open\)\) connect\(/);
  assert.match(src, /authWait\.refused\(\);\s*postMessage\(\{ connection: 'reauthenticating', authFailed: true \}\)/);
});

test('the core loads with the build version on both of its files', async () => {
  const { loadCore } = await import('../../static/player/reconnect.js');
  const asked = [];
  const fake = {
    default: async (o) => {
      asked.push(o.module_or_path);
      return { memory: 'mem' };
    },
    WasmPlayer: class {},
    WasmSegmenter: class {},
  };
  const core = await loadCore('https://music.example.com/player/player-worker.js?v=1791659999', async (u) => {
    asked.push(u);
    return fake;
  });
  assert.deepEqual(asked, [
    'https://music.example.com/player/pkg/anywhere_wasm.js?v=1791659999',
    'https://music.example.com/player/pkg/anywhere_wasm_bg.wasm?v=1791659999',
  ]);
  assert.equal(core.wasm.memory, 'mem');
  assert.equal(core.WasmSegmenter, fake.WasmSegmenter);
  // A missing or odd version is left out rather than passed through.
  asked.length = 0;
  await loadCore('https://music.example.com/player/player-worker.js?v=../../x', async (u) => (asked.push(u), fake));
  assert.equal(asked[0], 'https://music.example.com/player/pkg/anywhere_wasm.js');
});

test('the worker loads its helpers with the version and keeps early messages', () => {
  const src = readFileSync(new URL('../../static/player/player-worker.js', import.meta.url), 'utf8');
  assert.doesNotMatch(src, /^import .* from '\.\/(pkg|reconnect)/m, 'no unversioned static imports');
  assert.match(src, /await import\(\s*`\.\/reconnect\.js\$\{/);
  assert.match(src, /onmessage = \(e\) => early\.push\(e\)/);
  assert.match(src, /for \(const e of early\.splice\(0\)\) void onmessage\(e\);\s*$/);
});

test('seek timing records each stage once, relative to the flush, and finishes when heard', () => {
  const t = new SeekTimer();
  assert.equal(t.mark('stream', 5), null, 'nothing before a flush');
  t.start(1000);
  assert.equal(t.mark('stream', 1020), null);
  assert.equal(t.mark('stream', 1500), null, 'second mark ignored');
  t.mark('response', 1030.4);
  t.mark('bytes', 1031);
  t.mark('started', 1050);
  t.mark('queued', 1052);
  const done = t.mark('audible', 1080);
  assert.deepEqual(done, { total: 80, marks: { stream: 20, response: 30, bytes: 31, started: 50, queued: 52, audible: 80 } });
  assert.equal(t.active, false);
  assert.equal(t.mark('audible', 1100), null, 'finished');
  assert.equal(
    describeSeek(done),
    'seek: heard after 80 ms (new stream +20, response +30, first bytes +31, started +50, queued +52)',
  );
});

// Regression: audio of the old position, played just after the flush, counted as the seek
// being heard.
test('seek timing ignores audio heard before any new audio was queued', () => {
  const t = new SeekTimer();
  t.start(0);
  assert.equal(t.mark('audible', 4), null);
  assert.ok(t.active);
  t.mark('queued', 40);
  assert.equal(t.mark('audible', 60).total, 60);
});

test('seek timing gives up, restarts on a new flush, and ignores unknown stages', () => {
  const t = new SeekTimer();
  t.start(0);
  t.mark('queued', 10);
  assert.equal(t.mark('audible', SEEK_GIVE_UP_MS + 1), null, 'too late: dropped');
  assert.equal(t.active, false);
  t.start(100);
  t.start(200); // a second flush replaces the first
  t.mark('queued', 210);
  assert.equal(t.mark('audible', 250).total, 50);
  t.start(0);
  assert.equal(t.mark('__proto__', 1), null);
  assert.equal(t.mark('constructor', 1), null);
  assert.equal(t.mark('audible', NaN), null);
  t.cancel();
  assert.equal(t.active, false);
});

// Regression: MP3 seeks (flush and new stream ~2 ms apart) kept the old 8 s media queue,
// because the worker looked for a "Stopped" output state on its 20 ms tick.
test('flush watch reports each flush once, however the state looked in between', () => {
  const w = new FlushWatch();
  assert.equal(w.changed(0), false);
  assert.equal(w.changed(1), true);
  assert.equal(w.changed(1), false);
  assert.equal(w.changed(3), true, 'two flushes in one message still flush');
  w.reset();
  assert.equal(w.changed(0), false, 'a new core player starts at zero');
  assert.equal(w.changed(1), true);
});
