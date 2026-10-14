// A pause made on this device while the server is out of reach (node --test): the worker
// holds its output, keeps the position, and lets go once LMS has taken over again.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { LocalHold, RELEASE_DEADLINE_MS, SETTLE_MS, effectiveOutput, parseLocal } from '../../static/player/local.js';

test('hold and let go at once', () => {
  const h = new LocalHold();
  assert.equal(h.update('Playing', 0), false);
  h.hold();
  assert.equal(h.update('Playing', 10), true);
  // Held, the core's state changes nothing.
  assert.equal(h.update('Paused { until: None }', 20), true);
  h.release('now', 30);
  assert.equal(h.update('Playing', 30), false);
});

test('letting go of nothing is a no-op', () => {
  const h = new LocalHold();
  h.release('flush', 0);
  assert.equal(h.pending, null);
  assert.equal(h.update('Playing', 100_000), false);
});

test("'paused': held until LMS has paused the player too (not merely stopped it for a new stream)", () => {
  const h = new LocalHold();
  h.hold();
  h.release('paused', 0);
  // `playlist index` flushes (Stopped), then streams (Playing): still held.
  assert.equal(h.update('Stopped', 100), true);
  assert.equal(h.update('Waiting', 200), true);
  assert.equal(h.update('Playing', 300), true);
  assert.equal(h.update('Paused { until: None }', 400), false);
});

test("'flush': let go once LMS's flushes have settled", () => {
  const h = new LocalHold();
  h.hold();
  h.release('flush', 0);
  assert.equal(h.update('Playing', 50), true, 'no flush yet');
  h.onFlush(100); // `playlist index`
  assert.equal(h.update('Playing', 100 + SETTLE_MS - 1), true);
  h.onFlush(300); // then `time`: wait for this one to settle instead
  assert.equal(h.update('Playing', 100 + SETTLE_MS + 1), true);
  assert.equal(h.update('Playing', 300 + SETTLE_MS), false);
});

test('a flush before being told to let go does not count', () => {
  const h = new LocalHold();
  h.hold();
  h.onFlush(0);
  h.release('flush', 10);
  assert.equal(h.update('Playing', 10 + SETTLE_MS * 2), true);
});

test('never held past the deadline once told to let go (LMS did something else: follow it)', () => {
  for (const until of ['flush', 'paused']) {
    const h = new LocalHold();
    h.hold();
    h.release(until, 1000);
    assert.equal(h.update('Playing', 1000 + RELEASE_DEADLINE_MS - 1), true, until);
    assert.equal(h.update('Playing', 1000 + RELEASE_DEADLINE_MS), false, until);
  }
});

test('holding again cancels a pending release', () => {
  const h = new LocalHold();
  h.hold();
  h.release('paused', 0);
  h.hold();
  assert.equal(h.update('Paused { until: None }', 10), true);
  assert.equal(h.update('Playing', RELEASE_DEADLINE_MS * 2), true);
});

test('the outputs see "Held" while held, the core state otherwise', () => {
  assert.equal(effectiveOutput('Playing', true), 'Held');
  assert.equal(effectiveOutput('Playing', false), 'Playing');
  assert.equal(effectiveOutput('Paused { until: None }', false), 'Paused { until: None }');
});

test('messages from the page are checked; anything else is ignored', () => {
  assert.deepEqual(parseLocal({ cmd: 'local', hold: true }), { hold: true });
  for (const r of ['now', 'flush', 'paused']) assert.deepEqual(parseLocal({ cmd: 'local', release: r }), { release: r });
  const bad = [
    null,
    undefined,
    'hold',
    42,
    [],
    {},
    { hold: 'true' },
    { hold: 1 },
    { hold: false },
    { release: 'later' },
    { release: 'NOW' },
    { release: ['now'] },
    { release: { toString: () => 'now' } },
    { hold: true, release: 'now' }, // a mixture: neither
    { hold: false, release: 'now' },
    { release: '__proto__' },
    { release: 'constructor' },
  ];
  for (const m of bad) assert.equal(parseLocal(m), null, JSON.stringify(m));
});

test('robust: arbitrary messages and states never throw', () => {
  const h = new LocalHold();
  const values = [null, undefined, 0, -1, NaN, Infinity, '', 'x', {}, [], true, Symbol.iterator];
  for (const a of values) {
    assert.doesNotThrow(() => parseLocal(a));
    assert.doesNotThrow(() => parseLocal({ hold: a, release: a }));
    assert.doesNotThrow(() => h.update(a, 0));
    assert.doesNotThrow(() => effectiveOutput(a, false));
  }
  // Random sequences keep it consistent: held is a boolean, and a pending release only while held.
  let seed = 7;
  const rnd = () => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff);
  let t = 0;
  for (let i = 0; i < 5000; i++) {
    t += Math.floor(rnd() * 400);
    const r = rnd();
    if (r < 0.2) h.hold();
    else if (r < 0.4) h.release(['now', 'flush', 'paused'][Math.floor(rnd() * 3)], t);
    else if (r < 0.6) h.onFlush(t);
    else h.update(['Playing', 'Stopped', 'Paused { until: None }', 'Waiting'][Math.floor(rnd() * 4)], t);
    assert.equal(typeof h.held, 'boolean');
    if (!h.held) assert.equal(h.pending, null);
  }
});

test('the worker: held output is "Held" to both outputs, flushes are reported, status says so', () => {
  const src = readFileSync(new URL('../../static/player/player-worker.js', import.meta.url), 'utf8');
  assert.match(src, /await import\(`\.\/local\.js\$\{VQ\}`\)/);
  assert.match(src, /const output = effectiveOutput\(status\.output, localHold\.update\(status\.output, now\(\)\)\)/);
  assert.match(src, /mediaTick\(output\)/);
  assert.match(src, /holdWorklet\(output\)/);
  // The media element pauses for a hold like for LMS's pause; "drained" isn't the end while held.
  assert.match(src, /output === 'Loaded' \|\| output === 'Held'/);
  assert.match(src, /lastOutput = output;/);
  assert.match(src, /lastOutput === 'Playing'\) \{\s*\n\s*if \(segmenter\.pending_frames\(\)/);
  // Every flush is seen by the hold.
  assert.match(src, /function flushOutput\(\) \{[\s\S]*?localHold\.onFlush\(now\(\)\);[\s\S]*?\n\}/);
  // Page messages are checked, never trusted.
  assert.match(src, /msg\.cmd === 'local'\) \{\s*\n[^\n]*\n\s*const l = parseLocal\(msg\);/);
  assert.match(src, /status\.held = localHold\.held;/);
  assert.match(src, /status\.received = streamReceived;/);
  // The page learns whether a reconnection was a fresh session (what it did must be retold).
  assert.match(src, /postMessage\(\{ connection: 'connected', fresh,/);
});
