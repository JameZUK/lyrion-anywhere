// Partial flushes (LMS's `strm f`) at the web player's outputs (node --test), with the real
// WebAssembly core (built into static/player/pkg by tools/build-web.sh) and the real worklet.
//
// LMS sends `strm f` when the next track, already streamed, is deleted or moved in its
// queue, and keeps the current track playing. Audio of the dropped track already pulled
// into the output (the worklet's queue, or segments in the media element) must go too.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { planTrim } from '../../static/player/trim.js';

const pkg = new URL('../../static/player/pkg/', import.meta.url);
const core = await import(new URL('anywhere_wasm.js', pkg).href);
core.initSync({ module: readFileSync(new URL('anywhere_wasm_bg.wasm', pkg)) });

const RATE = 44_100;
const T1 = 1000; // each track's audio is one constant sample value, so it is recognisable
const T2 = 2000;
const trackOf = (sample) => Math.round(sample * 32768);

/** A `strm` server frame (see the core's proto::Strm). */
function strm(command) {
  const request = new TextEncoder().encode('GET /stream.wav?player=02:00:00:00:00:01 HTTP/1.0\r\n\r\n');
  const fixed = [command, 0x31, 0x70, 0x3f, 0x3f, 0x3f, 0x3f, 1, 0, 0, 0x30, 0, 0, 0, 0, 0, 0, 0, 0x23, 0x28, 0, 0, 0, 0];
  const payload = new Uint8Array([...new TextEncoder().encode('strm'), ...fixed, ...request]);
  const frame = new Uint8Array(2 + payload.length);
  new DataView(frame.buffer).setUint16(0, payload.length);
  frame.set(payload, 2);
  return frame;
}

/** 16-bit stereo WAV of `frames` frames of one sample value. */
function wav(frames, value) {
  const bytes = new Uint8Array(44 + frames * 4);
  const v = new DataView(bytes.buffer);
  bytes.set(new TextEncoder().encode('RIFF'), 0);
  v.setUint32(4, 36 + frames * 4, true);
  bytes.set(new TextEncoder().encode('WAVEfmt '), 8);
  v.setUint32(16, 16, true);
  v.setUint16(20, 1, true);
  v.setUint16(22, 2, true);
  v.setUint32(24, RATE, true);
  v.setUint32(28, RATE * 4, true);
  v.setUint16(32, 4, true);
  v.setUint16(34, 16, true);
  bytes.set(new TextEncoder().encode('data'), 36);
  v.setUint32(40, frames * 4, true);
  for (let i = 0; i < frames * 2; i++) v.setInt16(44 + i * 2, value, true);
  return bytes;
}

/** Track 1 playing, with track 2 fully streamed behind it. */
function twoTracks() {
  const p = new core.WasmPlayer('02:00:00:00:00:01', 'Web', 'pcm');
  p.set_host_volume(true);
  p.on_control(strm(0x73), 0);
  p.on_stream_data(wav(RATE, T1), 1);
  p.on_stream_end(2);
  for (let t = 3; t < 23; t++) p.tick(t);
  p.on_control(strm(0x73), 600);
  p.on_stream_data(wav(RATE, T2), 601);
  p.on_stream_end(602);
  for (let t = 603; t < 623; t++) p.tick(t);
  p.take_actions();
  return p;
}

function loadWorklet() {
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
  const workerPort = { onmessage: null, postMessage: () => {} };
  proc.port.onmessage({ data: { port: workerPort } });
  const send = (msg) => workerPort.onmessage({ data: msg });
  const render = (frames) => {
    const L = new Float32Array(frames);
    const R = new Float32Array(frames);
    proc.process([], [[L, R]]);
    return L;
  };
  return { send, render, proc };
}

test('Web Audio: after strm f the worklet plays the rest of track 1, then nothing of track 2', () => {
  const p = twoTracks();
  const { send, render } = loadWorklet();
  send({ flush: true, epoch: 0 });
  let pulled = 0;
  while (pulled < 50_000) {
    const buf = new Float32Array(2048 * 2);
    const n = p.pull(buf);
    assert.ok(n > 0);
    send({ pcm: buf.slice(0, n * 2) });
    pulled += n;
  }
  // 30 000 frames heard; track 1's last 14 100 and 5 900 of track 2 still queued.
  assert.ok(render(30_000).every((s) => trackOf(s) === T1));
  p.presented(30_000, 650);
  assert.equal(p.take_trim(), undefined, 'no trim yet');

  const flushes = p.flush_count();
  p.on_control(strm(0x66), 700); // strm f
  assert.equal(p.flush_count(), flushes, 'not a full flush');
  const t = p.take_trim();
  assert.ok(t, 'a trim');
  assert.equal(t.keep_frames, RATE - 30_000);
  assert.equal(t.drop_frames, pulled - RATE);
  assert.equal(p.take_trim(), undefined, 'once per strm f');

  const plan = planTrim(t, { mode: 'worklet', pulled });
  assert.deepEqual(plan.worklet, { trim: pulled - RATE });
  assert.equal(plan.pulled, RATE);
  send({ ...plan.worklet, epoch: 0 });

  const rest = render(20_000);
  for (let i = 0; i < RATE - 30_000; i++) assert.equal(trackOf(rest[i]), T1, `frame ${i}`);
  for (let i = RATE - 30_000; i < 20_000; i++) assert.equal(rest[i], 0, `frame ${i}: nothing of track 2`);
  assert.equal(p.pull(new Float32Array(4096)), 0, 'the core has nothing of track 2 either');
  p.free();
});

test('media element: after strm f the segments of track 2 are cut where track 1 ends', () => {
  const p = twoTracks();
  const seg = new core.WasmSegmenter(RATE);
  const buf = new Float32Array(4096 * 2);
  let pulled = 0;
  let emitted = 0;
  while (pulled < 60_000) {
    const n = p.pull_one(buf);
    assert.ok(n > 0);
    seg.push_track(buf.subarray(0, n * 2), p.pulled_track());
    while (seg.segment()) emitted++;
    pulled += n;
  }
  assert.ok(emitted > 0);
  p.presented(20_000, 650);
  p.on_control(strm(0x66), 700);
  const t = p.take_trim();
  assert.equal(t.drop_frames, pulled - RATE);

  const plan = planTrim(t, { mode: 'media', pulled, segmenter: seg, mediaWaiting: false });
  assert.equal(plan.pulled, RATE);
  assert.equal(plan.worklet, null);
  assert.equal(plan.media.length, 1, 'same rate: no new init segment');
  const blockStart = Math.floor(RATE / 4096) * 4096;
  const from = plan.media[0].trimFrom;
  assert.ok(from < blockStart / RATE && blockStart / RATE - from < 1 / RATE, `cut at ${from}`);
  // The timeline carries on there, with the rest of track 1 queued again.
  assert.ok(Math.abs(seg.emitted_seconds() - blockStart / RATE) < 1e-9);
  assert.equal(seg.pending_frames(), RATE - blockStart);
  assert.equal(p.pull_one(buf), 0, 'nothing of track 2 left to pull');
  seg.free();
  p.free();
});

test('a full flush is not a trim', () => {
  const p = twoTracks();
  p.pull(new Float32Array(50_000 * 2));
  p.on_control(strm(0x71), 700); // strm q
  assert.equal(p.take_trim(), undefined);
  p.free();
});

test('the worker trims its output after each control message, after the flush check', () => {
  const src = readFileSync(new URL('../../static/player/player-worker.js', import.meta.url), 'utf8');
  assert.match(src, /await import\(`\.\/trim\.js\$\{VQ\}`\)/, 'loaded with the build version');
  const onFrames = src.match(/function onFrames\(data\) \{[\s\S]*?\n\}/)?.[0] ?? '';
  const flushAt = onFrames.indexOf('flushWatch.changed(player.flush_count())');
  const trimAt = onFrames.indexOf('player.take_trim()');
  assert.ok(flushAt > 0 && trimAt > flushAt, 'take_trim follows the flush check');
  assert.match(onFrames, /if \(trim\) trimOutput\(trim\);/);
  const trimOutput = src.match(/function trimOutput\(t\) \{[\s\S]*?\n\}/)?.[0] ?? '';
  assert.match(trimOutput, /planTrim\(t, \{ mode: outputMode, pulled, segmenter, mediaWaiting \}\)/);
  assert.match(trimOutput, /visual\.trim\(pulled\)/);
  assert.match(trimOutput, /sink\.postMessage\(\{ \.\.\.plan\.worklet, epoch \}\)/);
  assert.match(trimOutput, /postMessage\(\{ media: m \}\)/);
  // The media element's audio is pulled a track at a time, and the segmenter told whose.
  const mediaTick = src.match(/function mediaTick\(output\) \{[\s\S]*?\n\}/)?.[0] ?? '';
  assert.match(mediaTick, /player\.pull_one\(buf\)/);
  assert.match(mediaTick, /segmenter\.push_track\(pcm, player\.pulled_track\(\)\)/);
  assert.doesNotMatch(mediaTick, /segmenter\.push\(/);
});

test('planTrim: worklet drops at most what was pulled', () => {
  assert.deepEqual(planTrim({ keep_track: 1, drop_frames: 500 }, { mode: 'worklet', pulled: 200 }), {
    drop: 200,
    pulled: 0,
    worklet: { trim: 200 },
    media: [],
  });
  assert.equal(planTrim({ keep_track: 1, drop_frames: 0 }, { mode: 'worklet', pulled: 200 }).worklet, null);
});

test('planTrim: hostile or missing values drop nothing and never throw', () => {
  for (const t of [null, undefined, 7, 'x', {}, { drop_frames: -5 }, { drop_frames: NaN }, { drop_frames: 1.5 }, { drop_frames: Infinity }, { drop_frames: '100' }]) {
    const plan = planTrim(t, { mode: 'worklet', pulled: 1000 });
    assert.equal(plan.drop, 0);
    assert.equal(plan.pulled, 1000);
    assert.equal(plan.worklet, null);
  }
  for (const pulled of [NaN, -1, undefined, 1.5]) assert.equal(planTrim({ drop_frames: 10 }, { mode: 'worklet', pulled }).drop, 0);
});

test('planTrim: media mode leaves the segmenter alone while a full flush is under way', () => {
  let trims = 0;
  const seg = {
    rate: () => RATE,
    trim_after: () => {
      trims++;
      return 1.5;
    },
  };
  assert.deepEqual(planTrim({ keep_track: 3, drop_frames: 10 }, { mode: 'media', pulled: 100, segmenter: seg, mediaWaiting: true }).media, []);
  assert.deepEqual(planTrim({ keep_track: -1, drop_frames: 10 }, { mode: 'media', pulled: 100, segmenter: seg }).media, []);
  assert.deepEqual(planTrim({ keep_track: 3, drop_frames: 10 }, { mode: 'media', pulled: 100, segmenter: null }).media, []);
  assert.equal(trims, 0);
  assert.deepEqual(planTrim({ keep_track: 3, drop_frames: 10 }, { mode: 'media', pulled: 100, segmenter: seg }).media, [{ trimFrom: 1.5 }]);
});

test('planTrim: media mode sends the init segment again when the rate goes back', () => {
  let rate = 96_000;
  const seg = {
    rate: () => rate,
    trim_after: () => {
      rate = RATE;
      return 2;
    },
    init_segment: () => new Uint8Array([1]),
    emitted_seconds: () => 2,
  };
  const { media } = planTrim({ keep_track: 3, drop_frames: 10 }, { mode: 'media', pulled: 100, segmenter: seg });
  assert.deepEqual(media, [{ trimFrom: 2 }, { init: new Uint8Array([1]), span: { at: 2, rate: RATE } }]);
});
