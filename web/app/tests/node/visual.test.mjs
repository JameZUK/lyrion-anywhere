// The visualiser's tap in the player worker (node --test): frames only while asked for, held
// back until the output has played that far, at most 30 a second, and dropped on a flush.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { FRAME_LEN, MAX_QUEUED, MIN_GAP_MS, RETAIN_SECS, VisualTap } from '../../static/player/visual.js';

/** A stand-in for the core's analyser: a frame every `hop` frames of audio, numbered. */
function fakeAnalyser(log = []) {
  return (rate, channels) => {
    const hop = Math.ceil(rate / 30);
    let since = 0;
    let n = 0;
    let pending = null;
    const v = {
      rate,
      channels,
      freed: false,
      resets: 0,
      push(pcm) {
        assert.equal(pcm.length % channels, 0);
        since += pcm.length / channels;
        while (since >= hop) {
          since -= hop;
          pending = new Float32Array(FRAME_LEN).fill(++n);
        }
      },
      take() {
        const f = pending;
        pending = null;
        return f;
      },
      reset() {
        this.resets++;
        since = 0;
        pending = null;
      },
      free() {
        this.freed = true;
      },
    };
    log.push(v);
    return v;
  };
}

const pcm = (frames) => new Float32Array(frames * 2);

test('nothing is analysed or sent until the page asks', () => {
  const made = [];
  const tap = new VisualTap(fakeAnalyser(made));
  tap.feed(pcm(48_000), 48_000, 0);
  assert.equal(made.length, 0);
  assert.equal(tap.due(1e9, 1000), null);
});

test('frames wait until the output has played that far', () => {
  const tap = new VisualTap(fakeAnalyser());
  tap.setOn(true);
  // One second pulled ahead at 48 kHz: 30 frames, one per 1600 frames of audio.
  tap.feed(pcm(48_000), 48_000, 0);
  assert.equal(tap.queue.length, 30);
  assert.equal(tap.due(1000, 0), null, 'not heard yet');
  const f = tap.due(1600, 100);
  assert.ok(f instanceof Float32Array);
  assert.equal(f[0], 1);
  // Too soon for another (≤ 30 fps), even though more is due.
  assert.equal(tap.due(48_000, 100 + MIN_GAP_MS - 1), null);
  // Later: the newest due one, older ones dropped.
  const g = tap.due(3 * 1600, 200);
  assert.equal(g[0], 3);
  assert.equal(tap.queue.length, 27);
});

test('turned on mid-track in media mode, it starts from what is about to be heard', () => {
  // Regression: the element plays seconds behind what the worker pulls, so a visualiser
  // turned on mid-track had nothing due until the newly pulled audio was heard (up to 8 s).
  const made = [];
  const tap = new VisualTap(fakeAnalyser(made));
  // Playing, visualiser off: 8 s pulled ahead of the element, kept by reference.
  for (let s = 0; s < 8; s++) tap.feed(pcm(48_000), 48_000, s * 48_000, true);
  assert.equal(made.length, 0, 'nothing analysed while off');
  tap.setOn(true);
  // The element is at 0.5 s: a frame is due at once.
  assert.ok(tap.due(24_000, 0) instanceof Float32Array);
  // Without retain (Web Audio: its chunks are handed to the worklet), nothing is kept.
  const wa = new VisualTap(fakeAnalyser());
  wa.feed(pcm(48_000), 48_000, 0);
  wa.setOn(true);
  assert.equal(wa.queue.length, 0);
});

test('what is kept while off is bounded, and a flush drops it', () => {
  const tap = new VisualTap(fakeAnalyser());
  for (let s = 0; s < 60; s++) tap.feed(pcm(48_000), 48_000, s * 48_000, true);
  assert.ok(tap.aheadFrames <= 48_000 * (RETAIN_SECS + 1), `${tap.aheadFrames}`);
  tap.flush();
  assert.equal(tap.ahead.length, 0);
  tap.setOn(true);
  assert.equal(tap.queue.length, 0);
});

test('a long chunk still gives a frame for every thirtieth of a second', () => {
  const tap = new VisualTap(fakeAnalyser());
  tap.setOn(true);
  // The media element's pulls are 4096 frames: more than one frame's worth each.
  for (let at = 0; at < 44_100; at += 4096) tap.feed(pcm(4096), 44_100, at);
  assert.ok(tap.queue.length >= 29, `${tap.queue.length}`);
  for (let i = 1; i < tap.queue.length; i++) assert.ok(tap.queue[i].at > tap.queue[i - 1].at);
});

test('a flush drops what was queued and resets the analyser', () => {
  const made = [];
  const tap = new VisualTap(fakeAnalyser(made));
  tap.setOn(true);
  tap.feed(pcm(48_000), 48_000, 0);
  tap.flush();
  assert.equal(tap.queue.length, 0);
  assert.equal(made[0].resets, 1);
  assert.equal(tap.due(1e9, 1000), null);
});

test('a new rate builds a new analyser; off frees it', () => {
  const made = [];
  const tap = new VisualTap(fakeAnalyser(made));
  tap.setOn(true);
  tap.feed(pcm(4096), 44_100, 0);
  tap.feed(pcm(4096), 96_000, 4096);
  assert.equal(made.length, 2);
  assert.equal(made[0].freed, true);
  assert.equal(made[1].rate, 96_000);
  assert.equal(made[1].channels, 2);
  tap.setOn(false);
  assert.equal(made[1].freed, true);
  assert.equal(tap.vis, null);
  assert.equal(tap.queue.length, 0);
});

test('the queue is bounded when the output stalls', () => {
  const tap = new VisualTap(fakeAnalyser());
  tap.setOn(true);
  for (let i = 0; i < 40; i++) tap.feed(pcm(48_000), 48_000, i * 48_000);
  assert.equal(tap.queue.length, MAX_QUEUED);
});

test('odd input never throws', () => {
  const tap = new VisualTap(fakeAnalyser());
  tap.setOn('yes'); // only true turns it on
  assert.equal(tap.on, false);
  tap.setOn(true);
  for (const [p, r, at] of [
    [null, 48_000, 0],
    [new Float32Array(0), 48_000, 0],
    [new Float32Array(1), 48_000, 0],
    [new Float32Array(3), 48_000, 0],
    [pcm(10), 0, 0],
    [pcm(10), NaN, 0],
    [pcm(10), 48_000, NaN],
  ]) {
    tap.feed(p, r, at);
  }
  assert.equal(tap.due(NaN, 0), null);
  // An analyser that fails to build, or gives frames of the wrong size: nothing is sent.
  const broken = new VisualTap(() => {
    throw new Error('no core yet');
  });
  broken.setOn(true);
  broken.feed(pcm(48_000), 48_000, 0);
  assert.equal(broken.due(1e9, 0), null);
  const wrong = new VisualTap(() => ({ push() {}, take: () => new Float32Array(3), reset() {} }));
  wrong.setOn(true);
  wrong.feed(pcm(48_000), 48_000, 0);
  assert.equal(wrong.due(1e9, 0), null);
});

test('the worker feeds both outputs and flushes with the output', () => {
  const src = readFileSync(new URL('../../static/player/player-worker.js', import.meta.url), 'utf8');
  // Web Audio and the media element both feed the tap with the PCM they send out.
  assert.equal(src.match(/visual\.feed\(/g)?.length, 2);
  assert.match(src, /function flushOutput\(\) \{\n {2}pulled = played = 0;\n {2}visual\.flush\(\);/);
  assert.match(src, /cmd === 'visual'/);
  assert.match(src, /visual\.due\(played, now\(\)\)/);
});
