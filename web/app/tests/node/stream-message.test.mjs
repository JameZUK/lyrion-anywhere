// Protocol 2 in the player worker (docs/PROTOCOL.md, *Versions* and *Messages after the
// greeting*): the stream message is checked strictly and tied to its stream by `seq`, which
// is the core's track number plus one, and close 4426 is never retried (node --test).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { STREAM_MESSAGE_MAX, StreamInfo, classifyText, factsSeq, parseStreamMessage, routeMessage } from '../../static/player/reconnect.js';

const GOOD = { stream: { seq: 12, served: { codec: 'flac', rate: 96000, bits: 24, channels: 2, gain_db: -3.2 }, sent: { mode: 'opus', kbps: 160, level: 'high' } } };
const withSeq = (seq, sent = GOOD.stream.sent) => ({ stream: { ...GOOD.stream, seq, sent } });

test('the stream message: the documented examples are taken as they are', () => {
  assert.deepEqual(classifyText(JSON.stringify(GOOD)), { stream: GOOD.stream });
  const pass = { stream: { seq: 1, served: { codec: 'mp3', rate: null, bits: null, channels: null }, sent: { mode: 'passthrough' } } };
  assert.deepEqual(classifyText(JSON.stringify(pass)), { stream: pass.stream });
  const low = { stream: { seq: 3, served: { codec: 'flac', rate: null, bits: null, channels: null }, sent: { mode: 'opus', kbps: 96, level: 'low' } } };
  assert.deepEqual(parseStreamMessage(low), low.stream);
  // Unknown fields inside are dropped, never passed on; absent numbers are null.
  const extra = { stream: { seq: 2, served: { codec: 'pcm', rate: 44100, bits: 16, channels: 2, evil: '<b>x</b>' }, sent: { mode: 'passthrough', kbps: 5 }, extra: 1 } };
  assert.deepEqual(parseStreamMessage(extra), { seq: 2, served: { codec: 'pcm', rate: 44100, bits: 16, channels: 2 }, sent: { mode: 'passthrough' } });
  assert.deepEqual(parseStreamMessage({ stream: { seq: 4, served: {}, sent: { mode: 'passthrough' } } }), {
    seq: 4,
    served: { codec: null, rate: null, bits: null, channels: null },
    sent: { mode: 'passthrough' },
  });
  // The result is a fresh object: the parsed message can't reach the page by reference.
  const parsed = parseStreamMessage(GOOD);
  assert.notEqual(parsed.served, GOOD.stream.served);
});

test('the stream message: every malformed shape is ignored, never thrown on', () => {
  const mk = (patch) => {
    const m = structuredClone(GOOD);
    patch(m.stream, m);
    return m;
  };
  const bidi = String.fromCharCode(0x202e);
  const bad = [
    mk((s) => (s.seq = 0)),
    mk((s) => (s.seq = -1)),
    mk((s) => (s.seq = 1.5)),
    mk((s) => (s.seq = '12')),
    mk((s) => (s.seq = null)),
    mk((s) => delete s.seq),
    mk((s) => (s.seq = 2 ** 53)),
    mk((s) => (s.served = null)),
    mk((s) => (s.served = [])),
    mk((s) => delete s.served),
    mk((s) => (s.sent = 'opus')),
    mk((s) => delete s.sent),
    mk((s) => (s.served.codec = 'FLAC')),
    mk((s) => (s.served.codec = 'flac<script>')),
    mk((s) => (s.served.codec = 'a'.repeat(40))),
    mk((s) => (s.served.codec = '')),
    mk((s) => (s.served.codec = 3)),
    mk((s) => (s.served.codec = `${bidi}flac`)),
    mk((s) => (s.served.rate = -96000)),
    mk((s) => (s.served.rate = 96000.5)),
    mk((s) => (s.served.rate = '96000')),
    mk((s) => (s.served.rate = 1e12)),
    mk((s) => (s.served.bits = 0)),
    mk((s) => (s.served.bits = 65)),
    mk((s) => (s.served.channels = 1000)),
    mk((s) => (s.served.gain_db = 'loud')),
    mk((s) => (s.served.gain_db = 1000)),
    mk((s) => (s.sent.mode = 'mp3')),
    mk((s) => (s.sent.level = 'ultra')),
    mk((s) => delete s.sent.level),
    mk((s) => (s.sent.kbps = '160')),
    mk((s) => (s.sent.kbps = 0)),
    mk((s) => (s.sent.kbps = 99999)),
    mk((s) => delete s.sent.kbps),
    mk((_, m) => (m.other = 1)),
    { stream: null },
    { stream: [] },
    { stream: 12 },
  ];
  for (const m of bad) {
    const text = JSON.stringify(m);
    assert.equal(parseStreamMessage(JSON.parse(text)), null, text);
    assert.equal(classifyText(text), 'bad-stream', text);
  }
  // NaN and Infinity aren't JSON, but a hand-made object is checked all the same.
  assert.equal(parseStreamMessage(mk((s) => (s.served.gain_db = NaN))), null);
  assert.equal(parseStreamMessage(mk((s) => (s.served.rate = Infinity))), null);
  for (const v of [null, undefined, 3, 'x', [], [GOOD]]) assert.equal(parseStreamMessage(v), null);
  // Oversized: the gateway never sends more than 512 bytes.
  const big = mk((s) => (s.served.pad = 'x'.repeat(STREAM_MESSAGE_MAX)));
  assert.equal(classifyText(JSON.stringify(big)), 'bad-stream');
  // A greeting carrying a stream field is not a greeting.
  assert.equal(classifyText(JSON.stringify({ session: 's', secret: 'x', stream: GOOD.stream })), 'bad-stream');
  // Random mutations of a good message: never a throw, and anything taken is sane.
  let seed = 11;
  const rnd = () => (seed = (seed * 1103515245 + 12345) % 2 ** 31) / 2 ** 31;
  const good = JSON.stringify(GOOD);
  const alphabet = '{}[]":,0123456789.ea-';
  for (let i = 0; i < 3000; i++) {
    const chars = [...good];
    for (let n = 1 + Math.floor(rnd() * 4); n > 0; n--) chars[Math.floor(rnd() * chars.length)] = alphabet[Math.floor(rnd() * alphabet.length)];
    const r = classifyText(chars.join(''));
    assert.ok(r === null || r === 'bad-stream' || r === 'keepalive' || typeof r === 'object');
    if (r && typeof r === 'object' && r.stream) {
      assert.ok(Number.isSafeInteger(r.stream.seq) && r.stream.seq > 0);
      assert.ok(['passthrough', 'opus'].includes(r.stream.sent.mode));
    }
  }
});

test('routing: stream messages go to stream, malformed ones to badStream', () => {
  const seen = [];
  const to = {
    frames: () => seen.push('frames'),
    greeting: () => seen.push('greeting'),
    stream: (s) => seen.push(`stream ${s.seq}`),
    badStream: () => seen.push('bad'),
    ignored: () => seen.push('ignored'),
  };
  routeMessage(JSON.stringify(GOOD), to);
  routeMessage('{"stream":{"seq":"x"}}', to);
  routeMessage('{"other":1}', to);
  routeMessage('{"keepalive":true}', to);
  assert.deepEqual(seen, ['stream 12', 'bad', 'ignored']);
  // A router without badStream falls back to ignored.
  const old = { frames() {}, greeting() {}, ignored: () => seen.push('ignored') };
  routeMessage('{"stream":{}}', old);
  assert.equal(seen.at(-1), 'ignored');
});

test("the gateway's echo of a level: exactly one key, one of the three levels; anything else ignored", () => {
  for (const q of ['original', 'high', 'low']) assert.deepEqual(classifyText(JSON.stringify({ quality: q })), { quality: q });
  const junk = [
    { quality: 'ultra' },
    { quality: 'HIGH' },
    { quality: '' },
    { quality: null },
    { quality: 1 },
    { quality: ['high'] },
    { quality: { level: 'high' } },
    { quality: 'high', extra: 1 },
    { quality: 'high', session: 's', secret: 'x' },
    { quality: 'high', stream: GOOD.stream },
  ];
  for (const m of junk) assert.equal(classifyText(JSON.stringify(m)), null, JSON.stringify(m));
  // An own __proto__ key is a second key.
  assert.equal(classifyText('{"quality":"high","__proto__":{"x":1}}'), null);
  assert.equal(classifyText('{"quality":"toString"}'), null);
  // Routed to `quality`; junk to `ignored`; a router without `quality` ignores it.
  const seen = [];
  const to = { frames() {}, greeting: () => seen.push('greeting'), quality: (q) => seen.push(`quality ${q}`), ignored: () => seen.push('ignored') };
  routeMessage('{"quality":"low"}', to);
  routeMessage('{"quality":"lowest"}', to);
  routeMessage('{"quality":"high"}', { frames() {}, greeting() {}, ignored: () => seen.push('ignored') });
  assert.deepEqual(seen, ['quality low', 'ignored', 'ignored']);
  // Robustness: random mutations never throw, and anything taken is one of the levels.
  let seed = 7;
  const rnd = () => (seed = (seed * 1103515245 + 12345) % 2 ** 31) / 2 ** 31;
  const good = '{"quality":"original"}';
  const alphabet = '{}[]":,lowhigrna_ ';
  for (let i = 0; i < 3000; i++) {
    const chars = [...good];
    for (let n = 1 + Math.floor(rnd() * 3); n > 0; n--) chars[Math.floor(rnd() * chars.length)] = alphabet[Math.floor(rnd() * alphabet.length)];
    const r = classifyText(chars.join(''));
    if (r && typeof r === 'object' && 'quality' in r) assert.ok(['original', 'high', 'low'].includes(r.quality));
  }
});

// --- Against the real WebAssembly core (built into static/player/pkg by tools/build-web.sh) ---

const pkg = new URL('../../static/player/pkg/', import.meta.url);
const core = await import(new URL('anywhere_wasm.js', pkg).href);
core.initSync({ module: readFileSync(new URL('anywhere_wasm_bg.wasm', pkg)) });
const RATE = 44_100;

/** A `strm` server frame (see the core's proto::Strm); `format` is LMS's codec letter. */
function strm(command, format = 0x70) {
  const request = new TextEncoder().encode('GET /stream.wav?player=02:00:00:00:00:01 HTTP/1.0\r\n\r\n');
  const fixed = [command, 0x31, format, 0x3f, 0x3f, 0x3f, 0x3f, 1, 0, 0, 0x30, 0, 0, 0, 0, 0, 0, 0, 0x23, 0x28, 0, 0, 0, 0];
  const payload = new Uint8Array([...new TextEncoder().encode('strm'), ...fixed, ...request]);
  const frame = new Uint8Array(2 + payload.length);
  new DataView(frame.buffer).setUint16(0, payload.length);
  frame.set(payload, 2);
  return frame;
}

/** Stereo PCM WAV of `frames` frames at `bits` (16 or 24). */
function wav(frames, bits = 16) {
  const bps = bits / 8;
  const bytes = new Uint8Array(44 + frames * 2 * bps);
  const v = new DataView(bytes.buffer);
  bytes.set(new TextEncoder().encode('RIFF'), 0);
  v.setUint32(4, 36 + frames * 2 * bps, true);
  bytes.set(new TextEncoder().encode('WAVEfmt '), 8);
  v.setUint32(16, 16, true);
  v.setUint16(20, 1, true);
  v.setUint16(22, 2, true);
  v.setUint32(24, RATE, true);
  v.setUint32(28, RATE * 2 * bps, true);
  v.setUint16(32, 2 * bps, true);
  v.setUint16(34, bits, true);
  bytes.set(new TextEncoder().encode('data'), 36);
  v.setUint32(40, frames * 2 * bps, true);
  return bytes;
}

/**
 * A session as the gateway and the worker see it: the gateway counts every `strm s` it
 * relays from 1 and announces each stream by that number; the worker hands each frame to the
 * core once.
 */
function session() {
  const p = new core.WasmPlayer('02:00:00:00:00:01', 'Web', 'pcm');
  p.set_host_volume(true);
  let relayed = 0;
  let t = 0;
  const opened = [];
  const take = () => {
    for (const a of p.take_actions()) if (a.type === 'open') opened.push(a.stream);
  };
  return {
    p,
    opened,
    /** A `strm s` relayed: returns the gateway's number for it. */
    start(format = 0x70) {
      p.on_control(strm(0x73, format), ++t);
      take();
      return ++relayed;
    },
    other(command) {
      p.on_control(strm(command), ++t);
      take();
    },
    /** The stream's audio, all of it. */
    feed(frames, bits) {
      p.on_stream_data(wav(frames, bits), ++t);
      p.on_stream_end(++t);
      for (let i = 0; i < 20; i++) p.tick(++t);
      take();
    },
    /** Pull everything decoded into the output, a track at a time (as the media path does). */
    pullAll() {
      const buf = new Float32Array(8192 * 2);
      while (p.pull_one(buf) > 0);
    },
  };
}

/** The seq the worker looks up for the badge (player-worker.js, tick). */
const seqNow = (p) => factsSeq(p.heard_track(), p.pulled_track(), null);

test("the core's track numbers are the gateway's stream numbers: a seek, a track change, a refused strm s and a resume", () => {
  const s = session();
  const { p } = s;
  // Track 1.
  const first = s.start();
  assert.equal(first, 1);
  assert.deepEqual(s.opened, [1], 'the stream URL carries the same number');
  s.feed(RATE);
  p.pull_one(new Float32Array(1000 * 2));
  p.played(1000);
  assert.equal(seqNow(p), first);
  // A seek: strm q, then strm s for the same track from a new place.
  s.other(0x71);
  const sought = s.start();
  s.feed(RATE);
  assert.equal(seqNow(p), sought, 'the new stream is the one about to be heard');
  p.pull_one(new Float32Array(1000 * 2));
  p.played(1000);
  assert.equal(seqNow(p), sought);
  // The next track, once this one has been fetched.
  s.pullAll();
  const next = s.start();
  s.feed(RATE);
  s.pullAll();
  p.played(2 * RATE);
  assert.equal(seqNow(p), next);
  // A strm s the core refuses (a codec it has no decoder for): still numbered by both sides.
  s.other(0x71);
  const before = s.opened.length;
  const refused = s.start(0x7a);
  assert.equal(s.opened.length, before, 'nothing is fetched for it');
  const after = s.start();
  assert.equal(after, refused + 1);
  assert.equal(s.opened.at(-1), after, 'the stream after it keeps the count');
  s.feed(RATE);
  p.pull_one(new Float32Array(1000 * 2));
  p.played(1000);
  assert.equal(seqNow(p), after);
  // A resume keeps the same core (the worker makes a new one only for a fresh session), and the
  // gateway replays only the frames after `received`: each frame reaches the core once, so the
  // count carries on.
  s.pullAll();
  const resumed = s.start();
  assert.equal(s.opened.at(-1), resumed);
  s.feed(RATE);
  s.pullAll();
  p.played(RATE * 2);
  assert.equal(seqNow(p), resumed);
  assert.deepEqual(s.opened, [first, sought, next, after, resumed]);
  p.free();
});

test("a gapless boundary: the badge's inputs stay track A's until A is no longer heard", () => {
  const s = session();
  const { p } = s;
  const info = new StreamInfo();
  // A: 16-bit, passed through. B: 24-bit, re-encoded to Opus by the gateway.
  const a = s.start();
  info.add(parseStreamMessage({ stream: { seq: a, served: { codec: 'pcm', rate: RATE, bits: 16, channels: 2 }, sent: { mode: 'passthrough' } } }));
  s.feed(RATE, 16);
  const b = s.start();
  info.add(parseStreamMessage({ stream: { seq: b, served: { codec: 'pcm', rate: RATE, bits: 24, channels: 2 }, sent: { mode: 'opus', kbps: 160, level: 'high' } } }));
  s.feed(RATE, 24);
  // The media path pulls ahead: all of A and all of B are in the output, A still heard.
  s.pullAll();
  assert.equal(p.pulled_track() + 1, b, 'B is the track last pulled');
  const badge = () => {
    const status = JSON.parse(p.status_json());
    const seq = seqNow(p);
    return { source: status.source?.bits, served: info.of(seq)?.served.bits, sent: info.of(seq)?.sent.mode };
  };
  p.played(RATE - 1);
  assert.deepEqual(badge(), { source: 16, served: 16, sent: 'passthrough' }, "A's last frame: all A's");
  p.played(2);
  assert.deepEqual(badge(), { source: 24, served: 24, sent: 'opus' }, "B heard: all B's");
  p.free();
});

test('factsSeq: the track heard, else the one pulled, else the stream being fetched', () => {
  assert.equal(factsSeq(0, 3, 9), 1);
  assert.equal(factsSeq(undefined, 3, 9), 4);
  assert.equal(factsSeq(undefined, undefined, 9), 9);
  assert.equal(factsSeq(undefined, undefined, undefined), null);
  for (const bad of [-1, 1.5, NaN, '2', null]) assert.equal(factsSeq(bad, bad, bad), null, String(bad));
});

test('stream messages: one re-sent after a resume replaces the one for that seq; a fresh session forgets them', () => {
  const info = new StreamInfo();
  info.add(parseStreamMessage({ stream: { seq: 1, served: { codec: 'flac', rate: 96000, bits: 24, channels: 2 }, sent: { mode: 'passthrough' } } }));
  info.add(parseStreamMessage(withSeq(2)));
  assert.equal(info.of(1).sent.mode, 'passthrough');
  assert.equal(info.of(2).sent.mode, 'opus');
  assert.equal(info.of(3), null);
  info.add(parseStreamMessage(withSeq(2, { mode: 'opus', kbps: 96, level: 'low' })));
  assert.equal(info.of(2).sent.kbps, 96);
  info.reset();
  assert.equal(info.of(2), null);
});

test('only the last few stream messages are kept', () => {
  const info = new StreamInfo();
  for (let seq = 1; seq <= 20; seq++) info.add({ seq, served: {}, sent: { mode: 'passthrough' } });
  assert.equal(info.of(12), null);
  assert.ok(info.of(13));
  assert.ok(info.of(20));
});

test('the worker: protocol 2 wiring (stream messages by track, 4426)', () => {
  const src = readFileSync(new URL('../../static/player/player-worker.js', import.meta.url), 'utf8');
  // A fresh session forgets the messages; a resumed one keeps what it knew.
  assert.match(src, /received = 0;\s*streamInfo\.reset\(\);/);
  // The badge's facts are looked up for the track heard, the one whose `source` is reported.
  assert.match(src, /const seq = factsSeq\(player\.heard_track\(\), status\.track, stream\?\.n\);/);
  assert.doesNotMatch(src, /streamInfo\.frame/);
  // The gateway's echo of a level goes to the page; a confirmed change is sent even if unchanged.
  assert.match(src, /quality: \(level\) => postMessage\(\{ qualityApplied: level \}\)/);
  assert.match(src, /if \(q !== quality \|\| msg\.confirm === true\) \{\s*quality = q;\s*sendQuality\(\);/);
  assert.match(src, /stream: \(info\) => streamInfo\.add\(info\)/);
  // Malformed messages are noted once.
  assert.match(src, /if \(badStreamNoted\) return;\s*badStreamNoted = true;/);
  // The status carries what LMS served and what the gateway sent, beside the source.
  assert.match(src, /status\.served = info\?\.served \?\? null;/);
  assert.match(src, /status\.sent = info\?\.sent \?\? null;/);
  // 4426: reported with its code and reason, and never connected again.
  assert.match(src, /postMessage\(\{ connection: 'closed', code: CLOSE\.TOO_OLD, reason: 'update the app' \}\)/);
  assert.match(src, /function connect\(resume\) \{\s*if \(tooOld\) return;/);
});
