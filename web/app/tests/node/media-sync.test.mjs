// LMS's sync corrections in media-element mode (node --test): made in the audio before it is
// muxed, never by seeking or pausing the element, and reported to the core at once.
//
// The model: a fake core whose decoded audio carries its own position (each frame's sample is
// its frame number in the track, plus one, so silence is 0), a fake segmenter that is the
// element's timeline, and an element that plays it in real time. The page reports what the
// element played every 100 ms, as device.svelte.ts does.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { MAX_CORRECTION_MS, MediaCorrections } from '../../static/player/media-sync.js';

const RATE = 44_100;
const REPORT_MS = 100;
const LEAD_SECS = 8; // the worker's MEDIA_AHEAD_SECS
const ms = (n) => Math.round((n * RATE) / 1000);

class FakeCore {
  decoded = Infinity; // frames decoded so far (Infinity: as much as is asked for)
  next = 0; // next frame to pull
  elapsed = 0; // frames presented (what the core reports to LMS)
  at = 0;
  pull_one(buf) {
    const n = Math.max(0, Math.min(buf.length / 2, this.decoded - this.next));
    for (let i = 0; i < n; i++) buf[2 * i] = buf[2 * i + 1] = this.next + i + 1;
    this.next += n;
    return n;
  }
  presented(frames, at) {
    assert.ok(Number.isInteger(frames) && frames >= 0, `presented ${frames}`);
    this.elapsed += frames;
    this.at = at;
  }
}

class FakeSegmenter {
  out = []; // the element's timeline, one sample per frame
  push_track(pcm, track) {
    assert.ok(Number.isInteger(track));
    for (let i = 0; i < pcm.length; i += 2) this.out.push(pcm[i]);
  }
}

/** The worker, the page and the element together, ticking every 100 ms. */
class Rig {
  core = new FakeCore();
  seg = new FakeSegmenter();
  corr = new MediaCorrections();
  pulled = 0;
  played = 0; // what the worker has been told the element played
  head = 0; // the element's play head, in timeline frames
  now = 0;

  fill() {
    this.pulled += this.corr.dropDue(this.core, this.now);
    while (this.pulled - this.played < RATE * LEAD_SECS) {
      const buf = new Float32Array(4096 * 2);
      const n = this.core.pull_one(buf);
      if (!n) break;
      this.seg.push_track(buf.subarray(0, n * 2), 1);
      this.pulled += n;
    }
  }

  skip(n) {
    const d = this.corr.skip(n, RATE, this.core, this.now);
    this.pulled += d;
    this.played += d;
  }

  pause(n) {
    // Silence counts towards what is queued ahead, or every pause would lengthen the lead.
    this.pulled += this.corr.pause(n, RATE, this.seg, 1);
  }

  /** 100 ms pass: the element plays, the page reports it, the worker tops up. */
  step() {
    this.now += REPORT_MS;
    const frames = Math.min(ms(REPORT_MS), this.seg.out.length - this.head);
    this.head += frames;
    this.played += frames;
    this.core.presented(this.corr.report(frames), this.now);
    this.fill();
  }

  run(msecs) {
    for (let t = 0; t < msecs; t += REPORT_MS) this.step();
  }

  /** Frames of the track heard so far: the position of the last track frame played. */
  heard() {
    for (let i = this.head - 1; i >= 0; i--) if (this.seg.out[i] > 0) return this.seg.out[i];
    return 0;
  }
}

function playing() {
  const r = new Rig();
  r.fill();
  r.run(5000);
  assert.equal(r.core.elapsed, r.heard(), 'in step before any correction');
  return r;
}

test('a skip is reported at once and the skipped frames are never counted again', () => {
  const r = playing();
  const before = r.core.elapsed;
  r.skip(160);
  assert.equal(r.core.elapsed - before, ms(160), 'elapsed jumps by the skip at once');
  // The element plays on without a break, through the audio already appended...
  r.run(REPORT_MS);
  assert.equal(r.core.elapsed - before, ms(160) + ms(REPORT_MS), 'then advances at real time');
  r.run(LEAD_SECS * 1000 + 1000);
  // ...and once it is past the cut, what is reported is exactly what is heard.
  assert.equal(r.core.elapsed, r.heard());
  // The timeline never held the skipped audio: no frame of it was appended.
  const cut = r.seg.out.findIndex((v, i) => i > 0 && v - r.seg.out[i - 1] !== 1);
  assert.equal(r.seg.out[cut] - r.seg.out[cut - 1], ms(160) + 1);
});

test('a pause is reported at once, and the silence is not counted as track audio', () => {
  const r = playing();
  const before = r.core.elapsed;
  r.pause(200);
  r.run(200);
  assert.equal(r.core.elapsed, before, 'elapsed stands still for the pause at once');
  r.run(REPORT_MS);
  assert.equal(r.core.elapsed - before, ms(REPORT_MS), 'then advances at real time');
  r.run(LEAD_SECS * 1000 + 1000);
  assert.equal(r.core.elapsed, r.heard());
  assert.equal(r.seg.out.filter((v) => v === 0).length, ms(200), 'the silence is in the audio, not a stop');
  assert.equal(r.corr.debt, 0);
});

// LMS (player.sync) compares the members' play points every 2 s and asks one that is behind
// to skip. The iPad was 160 ms behind; seeking the element stalled it for about as long, so
// the next check found it just as far behind, for ever.
test('the LMS loop: one skip puts the player in step, and the next checks ask for nothing', () => {
  const r = playing();
  // The reference player is 160 ms ahead of this one.
  const asked = [];
  let ref = r.core.elapsed + ms(160);
  for (let check = 0; check < 10; check++) {
    const behind = Math.round(((ref - r.core.elapsed) * 1000) / RATE);
    if (behind > 30) {
      asked.push(behind);
      r.skip(behind);
    }
    r.run(2000);
    ref += ms(2000);
  }
  assert.deepEqual(asked, [160]);
});

test('a skip beyond what is decoded drops the rest as it arrives', () => {
  const r = playing();
  r.core.decoded = r.core.next + ms(50);
  const before = r.core.elapsed;
  r.skip(160);
  assert.equal(r.core.elapsed - before, ms(50));
  assert.equal(r.corr.pendingSkip, ms(110));
  r.core.decoded = Infinity;
  r.step();
  assert.equal(r.corr.pendingSkip, 0);
  r.run(LEAD_SECS * 1000 + 1000);
  assert.equal(r.core.elapsed, r.heard());
});

test('a flush forgets what was owed', () => {
  const c = new MediaCorrections();
  const core = new FakeCore();
  core.decoded = 0;
  c.skip(100, RATE, core, 0);
  c.pause(100, RATE, new FakeSegmenter(), 1);
  assert.ok(c.pendingSkip > 0 && c.debt > 0);
  c.reset();
  assert.equal(c.pendingSkip, 0);
  assert.equal(c.debt, 0);
  assert.equal(c.report(4410), 4410);
});

test('junk corrections and reports are refused, never thrown on', () => {
  const c = new MediaCorrections();
  const core = new FakeCore();
  const seg = new FakeSegmenter();
  for (const bad of [NaN, -1, 0, Infinity, MAX_CORRECTION_MS + 1, '160', null, undefined, {}]) {
    assert.equal(c.skip(bad, RATE, core, 0), 0);
    assert.equal(c.pause(bad, RATE, seg, 1), 0);
  }
  for (const rate of [0, -44_100, NaN, 44_100.5, 2e6, '44100']) {
    assert.equal(c.skip(100, rate, core, 0), 0);
    assert.equal(c.pause(100, rate, seg, 1), 0);
  }
  for (const track of [-1, 1.5, NaN, undefined, null]) assert.equal(c.pause(100, RATE, seg, track), 0);
  assert.equal(c.pause(100, RATE, null, 1), 0);
  assert.equal(core.elapsed, 0);
  assert.equal(seg.out.length, 0);
  for (const bad of [NaN, -5, 1.5, Infinity, '10', null]) assert.equal(c.report(bad), 0);
  assert.equal(c.pendingSkip, 0);
  assert.equal(c.debt, 0);
});

test('random corrections never throw, never report negative frames, and add up', () => {
  let seed = 7;
  const rnd = () => ((seed = (seed * 1_103_515_245 + 12_345) % 2 ** 31) / 2 ** 31);
  const r = playing();
  for (let i = 0; i < 300; i++) {
    const x = rnd();
    if (x < 0.05) r.skip(Math.round(rnd() * 400));
    else if (x < 0.1) r.pause(Math.round(rnd() * 400));
    r.step();
  }
  r.run(LEAD_SECS * 1000 + 2000);
  assert.ok(r.seg.out.length - r.head <= RATE * LEAD_SECS + 4096, 'the silence does not lengthen the lead');
  assert.equal(r.core.elapsed, r.heard());
});
