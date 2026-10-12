// Presentation tests (node --test): what the Web Audio player reports as audible.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { CLOCK_MAX_AGE_MS, MAX_RUNS, Presentation } from '../../static/player/presentation.js';

const T0 = 1_700_000_000_000; // an absolute time, in ms

test('without a clock, rendered audio counts as heard (the old behaviour)', () => {
  const p = new Presentation();
  p.add([[0, 1000, 0]]);
  assert.equal(p.take(T0), 1000);
  assert.equal(p.take(T0), 0, 'counted once');
});

// Bluetooth-sized latency: rendering is 250 ms ahead of what is heard.
test('frames count as heard only when the output timestamp says so', () => {
  const p = new Presentation();
  const rate = 48000;
  // Context time 1.0 s was heard at T0.
  p.setClock(1.0, T0, rate);
  // The worklet has rendered up to 1.25 s.
  p.add([[48000, 12000, 0]]);
  assert.equal(p.take(T0), 0, 'nothing of it heard yet');
  assert.equal(p.take(T0 + 100), 4800, '100 ms later, 100 ms of it');
  assert.equal(p.pending(), 7200);
  assert.equal(p.take(T0 + 1000), 7200);
});

test('gaps between runs (silence, a pause) are not counted', () => {
  const p = new Presentation();
  p.setClock(0, T0, 1000);
  p.add([
    [0, 100, 0],
    [300, 100, 0],
  ]);
  assert.equal(p.take(T0 + 200), 100, 'in the gap: only the first run');
  assert.equal(p.take(T0 + 350), 50);
});

test('skipped frames count with the audio after them', () => {
  const p = new Presentation();
  p.setClock(0, T0, 1000);
  p.add([[100, 50, 40]]);
  assert.equal(p.take(T0 + 99), 0);
  assert.equal(p.take(T0 + 100), 40);
  assert.equal(p.take(T0 + 120), 20);
});

test('a timed start maps to the context frame heard at that moment', () => {
  const p = new Presentation();
  assert.equal(p.frameFor(T0 + 200, T0), null, 'no clock: no exact start');
  p.setClock(2.0, T0, 44100);
  assert.equal(p.frameFor(T0 + 200, T0), Math.round(2.2 * 44100));
});

test('a stale clock is not used', () => {
  const p = new Presentation();
  p.setClock(0, T0, 1000);
  p.add([[0, 10_000, 0]]);
  assert.equal(p.take(T0 + CLOCK_MAX_AGE_MS + 1), 10_000, 'stale: back to counting what was rendered');
  assert.equal(p.frameFor(T0 + CLOCK_MAX_AGE_MS + 2), null);
});

test('reset forgets the clock and runs; dropRuns keeps the clock', () => {
  const p = new Presentation();
  p.setClock(0, T0, 1000);
  p.add([[0, 100, 0]]);
  p.dropRuns();
  assert.equal(p.pending(), 0);
  assert.ok(p.hasClock(T0));
  p.reset();
  assert.ok(!p.hasClock(T0));
});

// Clock drift between the audio clock and the page's: the page refreshes the clock, and
// each refresh is used from then on.
test('a refreshed clock corrects drift', () => {
  const p = new Presentation();
  const rate = 48000;
  p.setClock(0, T0, rate);
  p.add([[0, 48000 * 20, 0]]);
  // The audio clock runs 100 ppm slow: after 10 s, context time 9.999 s is heard.
  p.setClock(9.999, T0 + 10_000, rate);
  assert.equal(p.take(T0 + 10_000), Math.floor(9.999 * rate));
});

// Security/robustness: a confused or hostile worklet can't make it throw, count junk or grow.
test('malformed runs and clocks are ignored', () => {
  const p = new Presentation();
  assert.equal(p.setClock(NaN, T0, 48000), false);
  assert.equal(p.setClock(1, T0, -1), false);
  assert.equal(p.setClock(1, -5, 48000), false);
  p.add('junk');
  p.add([null, [1], [1, 2], ['0', 1, 0], [-1, 5, 0], [0, 1.5, 0], [0, 5, -1], [0, 1e12, 0], [0, 0, 0]]);
  assert.equal(p.pending(), 0);
  for (let i = 0; i < MAX_RUNS + 100; i++) p.add([[i * 10, 1, 0]]);
  assert.equal(p.pending(), MAX_RUNS, 'bounded');
});
