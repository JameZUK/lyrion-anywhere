// AudioWorklet sink tests (node --test). The worklet globals are stubbed so the real
// processor code runs unmodified.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

function loadProcessor() {
  let Registered = null;
  const ctx = {
    AudioWorkletProcessor: class {
      constructor() {
        this.port = { onmessage: null };
      }
    },
    registerProcessor: (_name, cls) => {
      Registered = cls;
    },
  };
  vm.runInNewContext(readFileSync(new URL('../../static/player/worklet.js', import.meta.url), 'utf8'), ctx);
  const proc = new Registered();
  const reports = [];
  const workerPort = { onmessage: null, postMessage: (m) => reports.push(m) };
  proc.port.onmessage({ data: { port: workerPort } });
  const send = (msg) => workerPort.onmessage({ data: msg });
  const render = (frames = 128) => {
    const L = new Float32Array(frames).fill(NaN);
    const R = new Float32Array(frames).fill(NaN);
    assert.equal(proc.process([], [[L, R]]), true, 'processor must stay alive');
    return { L, R };
  };
  // Messages from the page (the AudioWorkletNode's port), e.g. the volume.
  const page = (msg) => proc.port.onmessage({ data: msg });
  return { send, render, reports, page };
}

const stereo = (frames, start = 0) => {
  const b = new Float32Array(frames * 2);
  for (let i = 0; i < frames; i++) {
    b[2 * i] = start + i;
    b[2 * i + 1] = -(start + i);
  }
  return b;
};

test('plays queued chunks contiguously across chunk boundaries', () => {
  const { send, render } = loadProcessor();
  send({ pcm: stereo(100, 0) });
  send({ pcm: stereo(100, 100) });
  const a = render(128);
  const b = render(128);
  const left = [...a.L, ...b.L];
  for (let i = 0; i < 200; i++) assert.equal(left[i], i, `sample ${i}`);
  assert.equal(a.R[5], -5);
});

test('outputs silence (not garbage) on underrun', () => {
  const { send, render } = loadProcessor();
  send({ pcm: stereo(10) });
  const { L, R } = render(128);
  for (let i = 10; i < 128; i++) {
    assert.equal(L[i], 0);
    assert.equal(R[i], 0);
  }
  const silent = render(128);
  assert.ok(silent.L.every((s) => s === 0));
});

test('flush drops queued audio immediately', () => {
  const { send, render } = loadProcessor();
  send({ pcm: stereo(1000, 1) });
  render(128);
  send({ flush: true });
  assert.ok(render(128).L.every((s) => s === 0));
});

test('reports exactly the frames actually played', () => {
  const { send, render, reports } = loadProcessor();
  send({ pcm: stereo(3000) });
  for (let i = 0; i < 30; i++) render(128); // 3840 frames requested, 3000 available
  const total = reports.reduce((n, r) => n + r.played, 0);
  assert.ok(total <= 3000, 'never reports more than was played');
  assert.ok(total >= 3000 - 1024, 'reports within one batch of what was played');
  assert.ok(reports.every((r) => Number.isInteger(r.played) && r.played > 0));
});

test('single-channel output is mixed down, not right-channel-only', () => {
  let Registered = null;
  const ctx = {
    AudioWorkletProcessor: class {
      constructor() {
        this.port = { onmessage: null };
      }
    },
    registerProcessor: (_n, cls) => {
      Registered = cls;
    },
  };
  vm.runInNewContext(readFileSync(new URL('../../static/player/worklet.js', import.meta.url), 'utf8'), ctx);
  const proc = new Registered();
  const port = { onmessage: null, postMessage: () => {} };
  proc.port.onmessage({ data: { port } });
  // Left 0.5, right 0.25 → mono 0.375 (mixed down, not right-channel-only).
  const pcm = new Float32Array(128);
  for (let i = 0; i < 64; i++) {
    pcm[2 * i] = 0.5;
    pcm[2 * i + 1] = 0.25;
  }
  port.onmessage({ data: { pcm } });
  const mono = new Float32Array(80);
  assert.equal(proc.process([], [[mono]]), true);
  assert.equal(mono[0], 0.375);
  assert.equal(mono[63], 0.375);
  assert.equal(mono[64], 0, 'silence after underrun');
});

// Regression: frames were reported in batches of 1024, so the last partial batch of a
// playlist was never reported. The core then never saw the end, never sent STMu, and LMS
// showed the player as playing forever.
test('reports every played frame once the queue runs dry', () => {
  const { send, render, reports } = loadProcessor();
  send({ pcm: stereo(1500) });
  for (let i = 0; i < 20; i++) render(128); // 2560 frames rendered: queue drains part-way
  const played = reports.filter((r) => r.played).reduce((n, r) => n + r.played, 0);
  assert.equal(played, 1500, 'all 1500 frames reported, including the final partial batch');
});

test('still batches reports while audio is flowing', () => {
  const { send, render, reports } = loadProcessor();
  send({ pcm: stereo(48000) });
  for (let i = 0; i < 100; i++) render(128); // 12800 frames, queue never empties
  const msgs = reports.filter((r) => r.played);
  assert.ok(msgs.every((r) => r.played >= 1024), 'no tiny reports mid-stream');
  assert.ok(msgs.length <= 12800 / 1024 + 1);
});

const constant = (frames, l = 1, r = 1) => {
  const b = new Float32Array(frames * 2);
  for (let i = 0; i < frames; i++) {
    b[2 * i] = l;
    b[2 * i + 1] = r;
  }
  return b;
};

// The volume is applied after the queue: audio already queued is not stuck at the old level.
test('a volume change applies to audio already queued, ramped without a jump', () => {
  const { send, render, page } = loadProcessor();
  send({ pcm: constant(48000) });
  render(128);
  page({ gain: [0.5, 0.5] });
  const a = render(128);
  assert.ok(a.L[0] < 1 && a.L[0] > 0.99, 'starts moving at once, by a small step');
  for (let i = 1; i < 128; i++) assert.ok(a.L[i] <= a.L[i - 1], 'monotonic ramp');
  // 30 ms at 48 kHz = 1440 samples: done well within 16 blocks.
  let last;
  for (let i = 0; i < 16; i++) last = render(128);
  assert.equal(last.L[127], 0.5);
  assert.equal(last.R[127], 0.5);
});

test('instant volume (a new output) applies from the first sample', () => {
  const { send, render, page } = loadProcessor();
  page({ gain: [0.25, 0.25], instant: true });
  send({ pcm: constant(256) });
  const a = render(128);
  assert.equal(a.L[0], 0.25);
  assert.equal(a.R[127], 0.25);
});

test('left and right gains apply per channel (balance)', () => {
  const { send, render, page } = loadProcessor();
  page({ gain: [1, 0.25], instant: true });
  send({ pcm: constant(256) });
  const a = render(128);
  assert.equal(a.L[5], 1);
  assert.equal(a.R[5], 0.25);
});

// Security/robustness: a gain from a message is clamped, never amplifies, and junk is ignored.
test('hostile volume messages never amplify or produce NaN', () => {
  const { send, render, page } = loadProcessor();
  for (const g of [[NaN, NaN], [Infinity, 1], [-1, -1], [7, 7], ['1', '0'], [null], [], [{}, {}]]) {
    page({ gain: g, instant: true });
  }
  page({ gain: 'loud' });
  page(null);
  page(42);
  send({ pcm: constant(256) });
  const a = render(128);
  for (let i = 0; i < 128; i++) {
    assert.ok(Number.isFinite(a.L[i]) && Number.isFinite(a.R[i]));
    assert.ok(Math.abs(a.L[i]) <= 1 && Math.abs(a.R[i]) <= 1, 'never above full scale');
  }
  page({ gain: [7, -3], instant: true });
  send({ pcm: constant(256) });
  render(128);
  const b = render(128);
  assert.equal(b.L[0], 1, 'clamped to full scale');
  assert.equal(b.R[0], 0, 'clamped to silence');
});

// Regression: "played" reports for audio of the old position arrived after a flush and were
// counted against the new one. Reports now carry the flush epoch.
test('played reports carry the epoch of the last flush', () => {
  const { send, render, reports } = loadProcessor();
  send({ pcm: stereo(2048) });
  render(128);
  send({ flush: true, epoch: 3 });
  send({ pcm: stereo(256) });
  for (let i = 0; i < 4; i++) render(128);
  const after = reports.filter((r) => r.played);
  assert.ok(after.length > 0);
  assert.equal(after[after.length - 1].epoch, 3);
  assert.equal(after.filter((r) => r.epoch === 3).reduce((n, r) => n + r.played, 0), 256, 'only the new audio');
});

// --- Sync: what is reported, and corrections applied at the output -------------------------

// Plain arrays (the worklet's come from another realm, which strict deepEqual tells apart).
const runsOf = (reports) => JSON.parse(JSON.stringify(reports.flatMap((r) => r.runs ?? [])));

test('reports runs of context frames for what it rendered', () => {
  const { send, render, reports } = loadProcessor();
  send({ pcm: stereo(300) });
  for (let i = 0; i < 4; i++) render(128);
  // Contiguous audio from context frame 0 is one run; the underrun after it is not in it.
  assert.deepEqual(runsOf(reports), [[0, 300, 0]]);
});

test('hold renders silence and keeps the queue', () => {
  const { send, render, reports } = loadProcessor();
  send({ hold: true });
  send({ pcm: stereo(500, 1) });
  assert.ok(render(128).L.every((s) => s === 0));
  send({ hold: false });
  const a = render(128);
  assert.equal(a.L[0], 1, 'nothing was consumed while held');
  assert.deepEqual(runsOf(reports), []);
});

// The first audible frame lands exactly on the requested context frame, mid-block.
test('a timed start begins at the exact context frame', () => {
  const { send, render, reports } = loadProcessor();
  send({ pcm: stereo(1000, 1) });
  send({ startAt: 300 });
  render(128);
  render(128); // frames 0..255: silent
  const c = render(128); // frames 256..383: audio from 300
  assert.equal(c.L[43], 0);
  assert.equal(c.L[44], 1, 'first frame at context frame 300');
  for (let i = 0; i < 20; i++) render(128);
  assert.equal(runsOf(reports)[0][0], 300);
});

test('a late timed start drops what would already have played', () => {
  const { send, render, reports } = loadProcessor();
  for (let i = 0; i < 4; i++) render(128); // context frame 512
  send({ pcm: stereo(1000, 0) });
  send({ startAt: 412 });
  const a = render(128);
  assert.equal(a.L[0], 100, 'frame 100 plays at context frame 512');
  for (let i = 0; i < 10; i++) render(128);
  const [start, , skipped] = runsOf(reports)[0];
  assert.deepEqual([start, skipped], [512, 100], 'the dropped frames count as heard with the start');
});

test('inserted silence delays the audio without consuming it', () => {
  const { send, render } = loadProcessor();
  send({ pcm: stereo(1000, 1) });
  render(128);
  send({ silence: 200 });
  assert.ok(render(128).L.every((s) => s === 0));
  const b = render(128);
  assert.ok(b.L.slice(0, 72).every((s) => s === 0));
  assert.equal(b.L[72], 129, 'carries on where it stopped');
});

test('a skip drops queued audio, and the rest as it arrives', () => {
  const { send, render, reports } = loadProcessor();
  send({ pcm: stereo(100, 0) });
  send({ skip: 150 });
  render(128);
  send({ pcm: stereo(300, 100) });
  const b = render(128);
  assert.equal(b.L[0], 150);
  for (let i = 0; i < 5; i++) render(128);
  const counted = reports.reduce((n, r) => n + r.played, 0);
  assert.equal(counted, 400, 'skipped frames are reported with the played ones');
});

test('a flush cancels corrections meant for the old audio', () => {
  const { send, render } = loadProcessor();
  send({ pcm: stereo(1000, 1) });
  send({ skip: 500 });
  send({ silence: 500 });
  send({ flush: true, epoch: 1 });
  send({ pcm: stereo(100, 7) });
  assert.equal(render(128).L[0], 7);
});

// Security/robustness: junk corrections are ignored, never NaN or a stuck output.
test('hostile correction messages are ignored', () => {
  const { send, render } = loadProcessor();
  for (const bad of [-1, 1.5, NaN, Infinity, '100', null, {}, 1e12]) {
    send({ startAt: bad });
    send({ silence: bad });
    send({ skip: bad });
  }
  send({ hold: 'yes' });
  send({ pcm: stereo(256, 1) });
  const a = render(128);
  assert.equal(a.L[0], 1, 'still playing: nothing junk took effect');
  for (const s of a.L) assert.ok(Number.isFinite(s));
});

test('worker messages that are not objects are ignored', () => {
  const { send, render } = loadProcessor();
  send(null);
  send(5);
  send('pcm');
  const a = render(128);
  assert.ok(a.L.every((s) => s === 0));
});

// --- Partial flush (`strm f`): the tracks after the one playing are dropped ---------------

// Regression: the worklet played the queued audio of a track LMS had dropped.
test('a trim drops the last frames queued and keeps the rest playing', () => {
  const { send, render } = loadProcessor();
  send({ pcm: stereo(100, 0) });
  send({ pcm: stereo(100, 100) });
  send({ pcm: stereo(100, 200) });
  render(50); // partly into the first chunk
  send({ trim: 180, epoch: 0 }); // the whole last chunk and 80 frames of the middle one
  const a = render(128);
  for (let i = 0; i < 70; i++) assert.equal(a.L[i], 50 + i, `frame ${i}`);
  for (let i = 70; i < 128; i++) assert.equal(a.L[i], 0, `frame ${i} was dropped`);
  send({ pcm: stereo(10, 1000) });
  assert.equal(render(128).L[0], 1000, 'new audio follows on');
});

test('a trim within the chunk being played keeps what is before it', () => {
  const { send, render } = loadProcessor();
  send({ pcm: stereo(100, 0) });
  render(30);
  send({ trim: 50, epoch: 0 });
  const a = render(128);
  assert.equal(a.L[19], 49);
  assert.equal(a.L[20], 0);
  send({ trim: 1000, epoch: 0 }); // more than is queued: just empties it
  assert.ok(render(128).L.every((s) => s === 0));
});

test('a trim from before the last flush is ignored', () => {
  const { send, render } = loadProcessor();
  send({ flush: true, epoch: 2 });
  send({ pcm: stereo(100, 1) });
  send({ trim: 100, epoch: 1 });
  send({ trim: 100 });
  assert.equal(render(128).L[99], 100, 'the new audio is all there');
});

test('hostile trim messages are ignored', () => {
  const { send, render } = loadProcessor();
  send({ pcm: stereo(100, 1) });
  for (const bad of [-1, 1.5, NaN, Infinity, '100', null, {}, 1e12]) send({ trim: bad, epoch: 0 });
  const a = render(128);
  assert.equal(a.L[99], 100, 'nothing dropped');
  for (const s of a.L) assert.ok(Number.isFinite(s));
});
