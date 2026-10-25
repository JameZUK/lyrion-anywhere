// Protocol 2 in the player worker (docs/PROTOCOL.md, *Versions* and *Messages after the
// greeting*): the stream message is checked strictly and tied to its stream by `seq`, the
// `strm s` frames are counted once each, and close 4426 is never retried (node --test).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { STREAM_MESSAGE_MAX, StreamInfo, classifyText, isStreamStart, parseStreamMessage, routeMessage } from '../../static/player/reconnect.js';

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

/** A server → client Slimproto frame: length, opcode, payload. */
function frame(op, payload = [0]) {
  const body = [...new TextEncoder().encode(op), ...payload];
  return new Uint8Array([body.length >> 8, body.length & 0xff, ...body]).buffer;
}

test('strm s frames are recognised; others are not', () => {
  assert.ok(isStreamStart(frame('strm', [0x73, 0x31])));
  assert.ok(isStreamStart(new Uint8Array(frame('strm', [0x73]))));
  for (const f of [frame('strm', [0x71]), frame('strm', [0x66]), frame('audg', [0x73]), frame('strm', []), new ArrayBuffer(3), null, 'strm s']) {
    assert.equal(isStreamStart(f), false);
  }
});

test('stream messages are tied to streams by seq, across a seek, a track change and a resume', () => {
  const info = new StreamInfo();
  let received = 0;
  const recv = (f) => info.frame(++received, f);
  // Track 1 starts: its strm s is the session's first stream.
  recv(frame('audg'));
  assert.equal(recv(frame('strm', [0x73])), true);
  assert.equal(info.starts, 1);
  info.add(parseStreamMessage({ stream: { seq: 1, served: { codec: 'flac', rate: 96000, bits: 24, channels: 2 }, sent: { mode: 'passthrough' } } }));
  // A seek: strm q then strm s, the second stream; its message says Opus.
  recv(frame('strm', [0x71]));
  recv(frame('strm', [0x73]));
  assert.equal(info.starts, 2);
  info.add(parseStreamMessage(withSeq(2)));
  assert.equal(info.of(1).sent.mode, 'passthrough');
  assert.equal(info.of(2).sent.mode, 'opus');
  assert.equal(info.of(3), null);
  // The socket drops and resumes. The gateway replays only what came after `received`, and
  // even a frame seen again (the same place in the session) is never counted twice.
  assert.equal(info.frame(received, frame('strm', [0x73])), false);
  assert.equal(info.frame(received - 1, frame('strm', [0x73])), false);
  assert.equal(info.starts, 2);
  // After the greeting the gateway re-sends the message for the stream in progress: the
  // latest for that seq is kept.
  info.add(parseStreamMessage(withSeq(2, { mode: 'opus', kbps: 96, level: 'low' })));
  assert.equal(info.of(2).sent.kbps, 96);
  // The next track: the third stream.
  recv(frame('strm', [0x73]));
  assert.equal(info.starts, 3);
  // A fresh session counts from 1 again and forgets the old messages.
  info.reset();
  assert.equal(info.starts, 0);
  assert.equal(info.of(2), null);
  assert.equal(info.frame(1, frame('strm', [0x73])), true);
  assert.equal(info.starts, 1);
});

test('only the last few stream messages are kept', () => {
  const info = new StreamInfo();
  for (let seq = 1; seq <= 20; seq++) info.add({ seq, served: {}, sent: { mode: 'passthrough' } });
  assert.equal(info.of(12), null);
  assert.ok(info.of(13));
  assert.ok(info.of(20));
});

test('the worker: protocol 2 wiring (stream messages, strm counting, 4426)', () => {
  const src = readFileSync(new URL('../../static/player/player-worker.js', import.meta.url), 'utf8');
  // Counted by their place in the session, straight after the resume counter.
  assert.match(src, /function onFrames\(data\) \{\s*received\+\+;\s*streamInfo\.frame\(received, data\);/);
  // A fresh session starts counting again; a resumed one keeps what it knew.
  assert.match(src, /received = 0;\s*streamInfo\.reset\(\);/);
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
