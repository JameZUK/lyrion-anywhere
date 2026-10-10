// Reconnect policy tests (node --test).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { AuthWait, CLOSE, Outbox, RateRequest, backoffMs, closeAction, streamFailureAction } from '../../static/player/reconnect.js';
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
