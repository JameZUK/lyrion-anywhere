// Reconnect policy tests (node --test).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { CLOSE, Outbox, backoffMs, closeAction, streamFailureAction } from '../../static/player/reconnect.js';

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
