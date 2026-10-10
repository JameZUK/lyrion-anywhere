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
  return { send, render, reports };
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
