// Reconnect policy tests (node --test).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  AuthWait,
  CLOSE,
  FlushWatch,
  Outbox,
  RateRequest,
  SEEK_GIVE_UP_MS,
  SeekTimer,
  backoffMs,
  closeAction,
  describeSeek,
  cleanEndAction,
  confirmGivesUp,
  downloadStream,
  megabytes,
  readOrTick,
  READ_PARKED_NOTE_MS,
  streamFailureAction,
  streamPath,
  socketSilent,
  SOCKET_SILENCE_MS,
} from '../../static/player/reconnect.js';
import { readFileSync } from 'node:fs';

test('close codes map to the right recovery', () => {
  // Security-relevant: never retry with credentials the gateway rejected, and never fight
  // another connection that legitimately took over this device.
  assert.equal(closeAction(CLOSE.POLICY), 'stop');
  assert.equal(closeAction(CLOSE.REPLACED), 'stop');
  // Session gone: start afresh.
  assert.equal(closeAction(CLOSE.NO_SESSION), 'new-session');
  // Regression: 1001 started a new session at once. Browsers send it too when they close a
  // socket themselves, and a fresh HELO makes LMS restart the track (or stop, if paused).
  // Resume instead: the gateway refuses (4404) if the session really ended.
  assert.equal(closeAction(CLOSE.GOING_AWAY), 'resume');
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
  for (const s of [400, 403, 404, 410, 416]) assert.equal(streamFailureAction(s), 'fail', `${s}`);
  // Regression: an expired access token failed the track (LMS then skipped or stopped).
  // The first 401 renews the token and retries; a second one is final (revoked).
  assert.equal(streamFailureAction(401), 'renew');
  assert.equal(streamFailureAction(401, true), 'fail');
  for (const s of [400, 403, 404, 410, 416]) assert.equal(streamFailureAction(s, false), 'fail', `${s}`);
});

test('a socket silent for longer than four of LMS status polls counts as dead', () => {
  assert.ok(SOCKET_SILENCE_MS >= 4 * 5_000, 'never shorter than a few LMS polls');
  assert.ok(SOCKET_SILENCE_MS <= 30_000, 'well within the 60 s resume grace');
  assert.equal(socketSilent(1_000, 1_000 + SOCKET_SILENCE_MS), false);
  assert.equal(socketSilent(1_000, 1_001 + SOCKET_SILENCE_MS), true);
  assert.equal(socketSilent(0, 100, 50), true);
});

test('outbox is bounded and drops oldest first', () => {
  const o = new Outbox(3);
  for (const x of [1, 2, 3, 4, 5]) o.push(x);
  assert.equal(o.length, 3);
  assert.equal(o.dropped, 2);
  assert.deepEqual(o.drain(), [3, 4, 5]);
  assert.equal(o.length, 0);
});

// Regression: the worker asked for a new AudioContext on every 20 ms tick until the page
// confirmed it, and the page built one per request (an AudioContext storm).
test('a rate change is requested once, not on every tick', () => {
  const r = new RateRequest();
  let ready = 44100;
  const asks = [];
  const tick = (rate) => r.want(rate, ready) && asks.push(rate);
  for (let i = 0; i < 50; i++) tick(44100); // already running at this rate
  assert.deepEqual(asks, []);
  for (let i = 0; i < 50; i++) tick(96000); // 50 ticks before the page is ready
  assert.deepEqual(asks, [96000]);
  ready = 96000;
  for (let i = 0; i < 10; i++) tick(96000);
  for (let i = 0; i < 10; i++) tick(48000);
  assert.deepEqual(asks, [96000, 48000]);
  // Back to a rate asked for before, after another one: asked again.
  ready = 48000;
  tick(96000);
  assert.deepEqual(asks, [96000, 48000, 96000]);
  assert.equal(r.want(0, ready), false);
  r.reset();
  assert.equal(r.want(44100, ready), true);
});

// Regression: after an unauthorised close, the worker reconnected only on a token sent with
// reconnect:true, so a renewal retried after being offline (a plain token update) never
// brought the player back.
test('waiting for a new token: any token reconnects, once, and only while waiting', () => {
  const w = new AuthWait();
  assert.equal(w.token(false), false); // not waiting: a routine token update
  w.refused();
  assert.equal(w.token(true), false); // somehow connected already
  assert.equal(w.token(false), true);
  assert.equal(w.token(false), false); // only once
  w.refused();
  assert.equal(w.token(false), true);
});

test('the worker goes through both guards', () => {
  const src = readFileSync(new URL('../../static/player/player-worker.js', import.meta.url), 'utf8');
  // Every rate request is gated by RateRequest.
  const posts = src.match(/postMessage\(\{ rate \}\)/g) ?? [];
  assert.equal(posts.length, 1);
  assert.match(src, /if \(rateRequest\.want\(rate, readyRate\)\) postMessage\(\{ rate \}\)/);
  // A token reconnects through AuthWait, which the unauthorised close arms.
  assert.match(src, /if \(authWait\.token\(open\)\) connect\(/);
  assert.match(src, /authWait\.refused\(\);\s*postMessage\(\{ connection: 'reauthenticating', authFailed: true \}\)/);
});

test('the core loads with the build version on both of its files', async () => {
  const { loadCore } = await import('../../static/player/reconnect.js');
  const asked = [];
  const fake = {
    default: async (o) => {
      asked.push(o.module_or_path);
      return { memory: 'mem' };
    },
    WasmPlayer: class {},
    WasmSegmenter: class {},
    WasmVisualiser: class {},
    notAClass: 7,
  };
  const core = await loadCore('https://music.example.com/player/player-worker.js?v=1791659999', async (u) => {
    asked.push(u);
    return fake;
  });
  assert.deepEqual(asked, [
    'https://music.example.com/player/pkg/anywhere_wasm.js?v=1791659999',
    'https://music.example.com/player/pkg/anywhere_wasm_bg.wasm?v=1791659999',
  ]);
  assert.equal(core.wasm.memory, 'mem');
  assert.equal(core.WasmSegmenter, fake.WasmSegmenter);
  // Regression: the visualiser's class was left out, so the worker could never build it.
  assert.equal(core.WasmVisualiser, fake.WasmVisualiser);
  assert.equal(core.notAClass, undefined);
  // A missing or odd version is left out rather than passed through.
  asked.length = 0;
  await loadCore('https://music.example.com/player/player-worker.js?v=../../x', async (u) => (asked.push(u), fake));
  assert.equal(asked[0], 'https://music.example.com/player/pkg/anywhere_wasm.js');
});

test('the worker loads its helpers with the version and keeps early messages', () => {
  const src = readFileSync(new URL('../../static/player/player-worker.js', import.meta.url), 'utf8');
  assert.doesNotMatch(src, /^import .* from '\.\/(pkg|reconnect)/m, 'no unversioned static imports');
  assert.match(src, /await import\(\s*`\.\/reconnect\.js\$\{/);
  assert.match(src, /onmessage = \(e\) => early\.push\(e\)/);
  assert.match(src, /for \(const e of early\.splice\(0\)\) void onmessage\(e\);\s*$/);
});

test('seek timing records each stage once, relative to the flush, and finishes when heard', () => {
  const t = new SeekTimer();
  assert.equal(t.mark('stream', 5), null, 'nothing before a flush');
  t.start(1000);
  assert.equal(t.mark('stream', 1020), null);
  assert.equal(t.mark('stream', 1500), null, 'second mark ignored');
  t.mark('response', 1030.4);
  t.mark('bytes', 1031);
  t.mark('started', 1050);
  t.mark('queued', 1052);
  const done = t.mark('audible', 1080);
  assert.deepEqual(done, { total: 80, marks: { stream: 20, response: 30, bytes: 31, started: 50, queued: 52, audible: 80 } });
  assert.equal(t.active, false);
  assert.equal(t.mark('audible', 1100), null, 'finished');
  assert.equal(
    describeSeek(done),
    'seek: heard after 80 ms (new stream +20, response +30, first bytes +31, started +50, queued +52)',
  );
});

// Regression: audio of the old position, played just after the flush, counted as the seek
// being heard.
test('seek timing ignores audio heard before any new audio was queued', () => {
  const t = new SeekTimer();
  t.start(0);
  assert.equal(t.mark('audible', 4), null);
  assert.ok(t.active);
  t.mark('queued', 40);
  assert.equal(t.mark('audible', 60).total, 60);
});

test('seek timing gives up, restarts on a new flush, and ignores unknown stages', () => {
  const t = new SeekTimer();
  t.start(0);
  t.mark('queued', 10);
  assert.equal(t.mark('audible', SEEK_GIVE_UP_MS + 1), null, 'too late: dropped');
  assert.equal(t.active, false);
  t.start(100);
  t.start(200); // a second flush replaces the first
  t.mark('queued', 210);
  assert.equal(t.mark('audible', 250).total, 50);
  t.start(0);
  assert.equal(t.mark('__proto__', 1), null);
  assert.equal(t.mark('constructor', 1), null);
  assert.equal(t.mark('audible', NaN), null);
  t.cancel();
  assert.equal(t.active, false);
});

// Regression: MP3 seeks (flush and new stream ~2 ms apart) kept the old 8 s media queue,
// because the worker looked for a "Stopped" output state on its 20 ms tick.
test('flush watch reports each flush once, however the state looked in between', () => {
  const w = new FlushWatch();
  assert.equal(w.changed(0), false);
  assert.equal(w.changed(1), true);
  assert.equal(w.changed(1), false);
  assert.equal(w.changed(3), true, 'two flushes in one message still flush');
  w.reset();
  assert.equal(w.changed(0), false, 'a new core player starts at zero');
  assert.equal(w.changed(1), true);
});

// --- Streaming quality ----------------------------------------------------------------

import { QUALITIES, qualityMessage, sanitiseQuality, signInMessage } from '../../static/player/reconnect.js';

test('levels are checked: anything else is original', () => {
  assert.deepEqual(QUALITIES, ['original', 'high', 'low']);
  for (const q of QUALITIES) assert.equal(sanitiseQuality(q), q);
  for (const bad of ['HIGH', 'ultra', '', null, undefined, 1, {}, ['low'], 'low ', '__proto__']) {
    assert.equal(sanitiseQuality(bad), 'original', String(bad));
  }
});

test('the sign-in message: a bare token unless resuming or below original', () => {
  assert.equal(signInMessage('tok', null, 'original'), 'tok');
  assert.equal(signInMessage('tok', null, 'nonsense'), 'tok');
  assert.deepEqual(JSON.parse(signInMessage('t"k', null, 'low')), { token: 't"k', quality: 'low' });
  assert.deepEqual(JSON.parse(signInMessage('t', { session: 's', secret: 'x', received: 4 }, 'high')), {
    token: 't',
    resume: { session: 's', secret: 'x', received: 4 },
    quality: 'high',
  });
  // Only the expected fields of the resume are sent.
  const m = JSON.parse(signInMessage('t', { session: 's', secret: 'x', received: 4, extra: 'leak' }, 'original'));
  assert.deepEqual(m, { token: 't', resume: { session: 's', secret: 'x', received: 4 } });
});

test('the control message', () => {
  assert.equal(qualityMessage('low'), '{"quality":"low"}');
  assert.equal(qualityMessage('evil"}'), '{"quality":"original"}');
});

// --- LMS busy: text keepalives on the player socket (docs/PROTOCOL.md) -------------------

import { classifyText, routeMessage } from '../../static/player/reconnect.js';

test('the sign-in asks for keepalives only when told to', () => {
  assert.equal(signInMessage('tok', null, 'original', false), 'tok');
  assert.equal(signInMessage('tok', null, 'original'), 'tok');
  assert.equal(signInMessage('tok', null, 'original', 'yes'), 'tok'); // only a real true
  assert.deepEqual(JSON.parse(signInMessage('tok', null, 'original', true)), { token: 'tok', keepalive: true });
  assert.deepEqual(JSON.parse(signInMessage('t', { session: 's', secret: 'x', received: 4 }, 'low', true)), {
    token: 't',
    resume: { session: 's', secret: 'x', received: 4 },
    quality: 'low',
    keepalive: true,
  });
});

test('a keepalive is exactly {"keepalive":true}', () => {
  assert.equal(classifyText('{"keepalive":true}'), 'keepalive');
  assert.equal(classifyText(' { "keepalive" : true } '), 'keepalive');
  for (const bad of ['{"keepalive":false}', '{"keepalive":1}', '{"keepalive":"true"}', '{"keepalive":null}']) {
    assert.equal(classifyText(bad), null, bad);
  }
});

test('a keepalive carrying greeting fields is neither a keepalive nor a greeting', () => {
  for (const f of ['session', 'secret', 'mac', 'name', 'resumed', 'grace']) {
    const m = JSON.stringify({ keepalive: true, [f]: 'x' });
    assert.equal(classifyText(m), null, m);
  }
  const mixed = { keepalive: true, session: 's', secret: 'x', mac: 'm', name: 'n', resumed: false };
  assert.equal(classifyText(JSON.stringify(mixed)), null);
});

test('the greeting is recognised by its session and secret', () => {
  const g = { session: 's1', secret: 'x', mac: '02:00:00:00:00:01', name: 'Test', resumed: false, grace: 60 };
  assert.deepEqual(classifyText(JSON.stringify(g)), { greeting: g });
  for (const bad of [{ session: 's1' }, { secret: 'x' }, { session: 1, secret: 'x' }, { session: 's', secret: null }, {}]) {
    assert.equal(classifyText(JSON.stringify(bad)), null, JSON.stringify(bad));
  }
});

test('junk text is ignored, never thrown on', () => {
  const junk = [
    '',
    'not json',
    '{',
    'null',
    'true',
    '42',
    '"keepalive"',
    '[{"keepalive":true}]',
    '[]',
    '{"__proto__":{"keepalive":true}}',
    'x'.repeat(5000),
    `{"keepalive":true,"pad":"${'x'.repeat(5000)}"}`,
  ];
  for (const j of junk) assert.equal(classifyText(j), null, j.slice(0, 40));
  for (const v of [undefined, null, 3, {}, new ArrayBuffer(2)]) assert.equal(classifyText(v), null);
  // Random strings from a JSON-ish alphabet: never a throw.
  let seed = 7;
  const rnd = () => (seed = (seed * 1103515245 + 12345) % 2 ** 31) / 2 ** 31;
  const alphabet = '{}[]":,truefalsnkepliv\\ 0123456789sesioncrt';
  for (let i = 0; i < 2000; i++) {
    let s = '';
    for (let n = Math.floor(rnd() * 40); n > 0; n--) s += alphabet[Math.floor(rnd() * alphabet.length)];
    const r = classifyText(s);
    assert.ok(r === null || r === 'keepalive' || typeof r === 'object');
  }
});

test('routing: a keepalive goes nowhere, frames are the only thing counted', () => {
  const seen = { frames: 0, greeting: 0, ignored: 0 };
  const to = {
    frames: () => seen.frames++,
    greeting: () => seen.greeting++,
    ignored: () => seen.ignored++,
  };
  routeMessage('{"keepalive":true}', to);
  routeMessage('{"keepalive":true}', to);
  assert.deepEqual(seen, { frames: 0, greeting: 0, ignored: 0 });
  routeMessage('{"keepalive":true,"session":"s","secret":"x"}', to);
  routeMessage('garbage', to);
  assert.deepEqual(seen, { frames: 0, greeting: 0, ignored: 2 });
  routeMessage(new ArrayBuffer(4), to);
  routeMessage('{"session":"s","secret":"x","mac":"m","name":"n","resumed":true}', to);
  assert.deepEqual(seen, { frames: 1, greeting: 1, ignored: 2 });
});

test('the worker: every message refreshes last heard; keepalives asked for only with lms_busy', () => {
  const src = readFileSync(new URL('../../static/player/player-worker.js', import.meta.url), 'utf8');
  // Every message is routed through routeMessage, after noting that the socket is alive.
  assert.match(src, /ws\.onmessage = \(m\) => \{[^}]*lastHeard = now\(\);\s*routeMessage\(m\.data, \{/);
  assert.doesNotMatch(src, /onGreeting\(JSON\.parse/);
  // Frames are counted in onFrames only.
  assert.equal((src.match(/received\+\+/g) ?? []).length, 1);
  assert.match(src, /function onFrames\(data\) \{\s*received\+\+;/);
  // Keepalives are asked for only when the gateway lists lms_busy.
  assert.match(src, /busyFeature = me\?\.features\?\.lms_busy === true;/);
  assert.match(src, /signInMessage\(token, resume \? \{ session, secret, received \} : null, quality, busyFeature === true\)/);
  // The features are learned (briefly) before the first sign-in.
  assert.match(src, /await learnFeatures\(\d+\);\s*connect\(false\);/);
});

// Review (Web nit): the session id went into the stream address unencoded.
test('the stream address encodes the session id and takes only whole numbers', () => {
  assert.equal(streamPath('abc123', 4, 0), '/v1/stream/abc123/4');
  assert.equal(streamPath('abc123', 4, 1000), '/v1/stream/abc123/4?offset=1000');
  // Whatever the session id holds, it stays one path segment: no climbing out, no query.
  assert.equal(streamPath('../../admin?x=1#y', 1, 0), '/v1/stream/..%2F..%2Fadmin%3Fx%3D1%23y/1');
  assert.equal(streamPath('a b/c', 2, 5), '/v1/stream/a%20b%2Fc/2?offset=5');
  for (const [n, off] of [[-1, 0], [1.5, 0], [NaN, 0], ['2', 0], [1, -5], [1, 2.5], [1, NaN]]) {
    const p = streamPath('s', n, off);
    assert.match(p, /^\/v1\/stream\/s\/\d+(\?offset=\d+)?$/, String([n, off]));
  }
  const src = readFileSync(new URL('../../static/player/player-worker.js', import.meta.url), 'utf8');
  const lib = readFileSync(new URL('../../static/player/reconnect.js', import.meta.url), 'utf8');
  assert.match(lib, /io\.fetch\(streamPath\(io\.session\(\), n, offset\)/);
  assert.match(src, /return downloadStream\(/);
  assert.doesNotMatch(src, /\/v1\/stream\//);
  assert.doesNotMatch(lib, /\/v1\/stream\/\$\{session\}/);
});

test('a clean end is confirmed only after a response that brought bytes', () => {
  assert.equal(cleanEndAction(0), 'end');
  assert.equal(cleanEndAction(1), 'confirm');
  assert.equal(cleanEndAction(4 << 20), 'confirm');
  // A confirmation that fails for good takes the end as the end, as before; network trouble
  // is retried first.
  assert.equal(confirmGivesUp('fail', 0, 12), true);
  assert.equal(confirmGivesUp('retry', 3, 12), false);
  assert.equal(confirmGivesUp('retry', 12, 12), true);
  assert.equal(confirmGivesUp('renew', 12, 12), false);
});

/**
 * A fake gateway for one stream of `total` bytes, and a fake core that takes everything. Each
 * request gets the bytes from its `?offset=` on, ending as `plans[i]` says for request i:
 * { upTo: n, end: 'clean' | 'error' } stops at byte n (cleanly, or with a broken connection),
 * { status } answers with that status, and no plan sends the rest and ends cleanly.
 */
function fakeDownload(total, plans) {
  const requests = [];
  const events = [];
  let have = 0;
  const player = {
    open_stream: () => [1, have],
    wants_stream_data: () => true,
    on_stream_headers: () => events.push('headers'),
    on_stream_data: (v) => (have += v.length),
    on_stream_end: () => events.push(`end@${have}`),
    on_stream_error: () => events.push(`error@${have}`),
  };
  const io = {
    player,
    current: () => true,
    fetch: async (path) => {
      const m = path.match(/offset=(\d+)/);
      const offset = m ? Number(m[1]) : 0;
      const plan = plans[requests.length] ?? {};
      requests.push(offset);
      if (plan.status) return { ok: false, status: plan.status, headers: new Map(), body: null };
      const stop = Math.min(plan.upTo ?? total, total);
      let pos = offset;
      return {
        ok: true,
        status: 200,
        headers: new Map([['content-type', 'audio/flac']]),
        body: {
          getReader: () => ({
            read: async () => {
              if (pos < stop) {
                const n = Math.min(1000, stop - pos);
                pos += n;
                return { done: false, value: new Uint8Array(n) };
              }
              if (plan.end === 'error') throw new TypeError('network error');
              if (plan.end === 'hang') return new Promise(() => {}); // never delivers again
              return { done: true, value: undefined };
            },
          }),
        },
      };
    },
    session: () => 's',
    token: () => 't',
    setController: () => {},
    sleep: async () => {},
    now: () => 0,
    log: (msg) => events.push(msg),
    mark: () => {},
    ack: (n, b) => events.push(`ack ${b}`),
    runActions: () => {},
    renewToken: async () => {},
    received: () => events.push('received'),
    maxTries: 12,
    ackEvery: 4000,
    readStallMs: 0, // no read watchdog unless a test asks for one
  };
  return { io, requests, events, have: () => have };
}

const ends = (events) => events.filter((e) => /^(end|error)@/.test(e));

// Issue #12: the gateway ends a stream whose client stopped acknowledging (its buffer full)
// with an error, so the player fetches the rest from `?offset=`. It must never take that as
// the end of the track, which skips to the next one.
test('a stream broken mid-way is fetched again from the offset reached, not ended', async () => {
  const d = fakeDownload(10_000, [{ upTo: 6000, end: 'error' }]);
  await downloadStream(d.io, 1);
  assert.deepEqual(d.requests, [0, 6000, 10_000]);
  assert.equal(d.have(), 10_000);
  assert.deepEqual(ends(d.events), ['end@10000']);
  assert.ok(d.events.some((e) => /interrupted at byte 6000/.test(e)));
});

// Issue #12, through a Cloudflare Tunnel: the gateway's error end can reach the browser as a
// clean end, which looked like the end of the track. Before the fix the track ended at byte
// 6000 and the rest was never fetched.
test('a clean end that came early (a proxy hid the error) is confirmed, and the rest fetched', async () => {
  const d = fakeDownload(10_000, [{ upTo: 6000, end: 'clean' }]);
  await downloadStream(d.io, 1);
  assert.deepEqual(d.requests, [0, 6000, 10_000]);
  assert.equal(d.have(), 10_000);
  assert.deepEqual(ends(d.events), ['end@10000']);
  assert.ok(d.events.includes('stream 1: clean end at byte 6000, confirming'), d.events.join('\n'));
  assert.ok(d.events.includes('stream 1: not the end; continued from byte 6000'));
  assert.ok(!d.events.includes('stream 1: end confirmed at byte 6000'));
  assert.ok(d.events.includes('stream 1: end confirmed at byte 10000'));
  assert.equal(d.events.filter((e) => e === 'received').length, 1);
});

test('the true end costs one empty request, and the end is reported once', async () => {
  const d = fakeDownload(10_000, []);
  await downloadStream(d.io, 1);
  assert.deepEqual(d.requests, [0, 10_000]);
  assert.deepEqual(ends(d.events), ['end@10000']);
  assert.ok(!d.events.some((e) => /resumed at byte/.test(e)));
  // An empty stream: nothing to confirm.
  const empty = fakeDownload(0, []);
  await downloadStream(empty.io, 1);
  assert.deepEqual(empty.requests, [0]);
  assert.deepEqual(ends(empty.events), ['end@0']);
});

test('a confirmation refused for good (an older gateway, a stream gone) takes the end as before', async () => {
  for (const status of [400, 404, 410, 416]) {
    const d = fakeDownload(10_000, [{}, { status }]);
    await downloadStream(d.io, 1);
    assert.deepEqual(d.requests, [0, 10_000], `${status}`);
    assert.deepEqual(ends(d.events), ['end@10000'], `${status}`);
  }
  // Network trouble while confirming is retried, then given up as the end, never a failure.
  const d = fakeDownload(10_000, [{}, ...Array(20).fill({ status: 503 })]);
  await downloadStream(d.io, 1);
  assert.equal(d.requests.length, 1 + 13);
  assert.deepEqual(ends(d.events), ['end@10000']);
});

test('repeated early clean ends each need progress: no loop; no progress still fails as before', async () => {
  const d = fakeDownload(10_000, [
    { upTo: 2000, end: 'clean' },
    { upTo: 4000, end: 'clean' },
    { upTo: 8000, end: 'clean' },
  ]);
  await downloadStream(d.io, 1);
  assert.deepEqual(d.requests, [0, 2000, 4000, 8000, 10_000]);
  assert.deepEqual(ends(d.events), ['end@10000']);
  // A stream that keeps breaking without progress fails after the retries.
  const stuck = fakeDownload(10_000, Array(20).fill({ upTo: 3000, end: 'error' }));
  await downloadStream(stuck.io, 1);
  assert.deepEqual(ends(stuck.events), ['error@3000']);
  assert.equal(stuck.requests.length, 13);
});

test('bytes are acknowledged as they arrive, across a resume', async () => {
  const d = fakeDownload(10_000, [{ upTo: 6000, end: 'error' }]);
  await downloadStream(d.io, 1);
  assert.deepEqual(
    d.events.filter((e) => e.startsWith('ack ')),
    ['ack 4000', 'ack 8000'],
  );
});

// Issue #12 diagnostics: the connection log says how each clean end was settled.
test('a clean end at the true end is logged as confirmed', async () => {
  const d = fakeDownload(10_000, []);
  await downloadStream(d.io, 1);
  assert.ok(d.events.includes('stream 1: clean end at byte 10000, confirming'), d.events.join('\n'));
  assert.ok(d.events.includes('stream 1: end confirmed at byte 10000'));
  // An empty stream ends at once: nothing to confirm, nothing logged about it.
  const empty = fakeDownload(0, []);
  await downloadStream(empty.io, 1);
  assert.ok(!empty.events.some((e) => /confirm/.test(e)));
});

/** A fake download whose core refuses data for the first `checks` back-pressure checks. */
function parkedDownload(checks, undecoded) {
  const d = fakeDownload(4000, []);
  let t = 0;
  let parked = checks;
  d.io.now = () => t;
  d.io.sleep = async (ms) => void (t += ms);
  if (undecoded) d.io.undecoded = undecoded;
  d.io.player.wants_stream_data = () => !(parked-- > 0);
  return d;
}

// Issue #12 diagnostics: a download held back by a full core buffer is logged once it has
// been parked for a while (and when it carries on), never at every 50 ms check.
test('feeding held up by a full core is logged once, with what the core and the worker hold', async () => {
  const d = parkedDownload(400, () => 6_900_000); // checks of 50 ms: well past the note
  await downloadStream(d.io, 1);
  const notes = d.events.filter((e) => /feeding/.test(e));
  assert.equal(notes.length, 2, notes.join('\n'));
  assert.match(notes[0], /^stream 1: feeding paused, core holds 6\.9 MB undecoded \(\d+\.\d MB more read\)$/);
  assert.match(notes[1], /^stream 1: feeding resumed after \d+ s$/);
  assert.equal(d.have(), 4000);
  // The response was read to its end while the core refused: nothing waited on the network.
  assert.ok(!d.events.some((e) => /reading/.test(e)));
});

test('a short hold-up is not logged; a failing undecoded() never breaks the download', async () => {
  const d = parkedDownload(Math.floor(READ_PARKED_NOTE_MS / 50) - 4);
  await downloadStream(d.io, 1);
  assert.ok(!d.events.some((e) => /feeding/.test(e)), d.events.join('\n'));
  assert.equal(d.have(), 4000);
  const e = parkedDownload(400, () => {
    throw new Error('no core');
  });
  await downloadStream(e.io, 1);
  assert.ok(e.events.some((x) => /^stream 1: feeding paused, core holds 0\.0 MB undecoded/.test(x)));
  assert.equal(e.have(), 4000);
});

// Issue #12 (iPad, 24/96 FLAC): Safari stopped delivering a response body it already held,
// with the core wanting data, and nothing in the player noticed: the track starved for good.
// A read that brings nothing for READ_STALL_MS is given up and the download resumed from
// the offset reached; the gateway replays from there.
test('a read that delivers nothing for too long is abandoned and the download resumed', async () => {
  const d = fakeDownload(9000, [{ upTo: 4000, end: 'hang' }]);
  d.io.readStallMs = 5;
  d.io.now = () => Date.now();
  d.io.sleep = (ms) => new Promise((r) => setTimeout(r, Math.min(ms, 20)));
  await downloadStream(d.io, 1);
  assert.deepEqual(d.requests, [0, 4000, 9000], 'resumed from the stall, then confirmed the end');
  assert.ok(d.events.some((e) => e === 'stream 1: no data for 0 s at byte 4000; resuming'), d.events.join('\n'));
  assert.ok(d.events.some((e) => /^stream 1 interrupted at byte 4000 \(read stalled\); retrying$/.test(e)));
  assert.deepEqual(ends(d.events), ['end@9000']);
});

test('a read in flight is returned when it lands, or null at the tick with the read kept', async () => {
  const landed = Promise.resolve({ done: false, value: new Uint8Array(1) });
  const neverSleep = () => new Promise(() => {});
  assert.deepEqual(await readOrTick(landed, 50, neverSleep), { done: false, value: new Uint8Array(1) });
  const hung = new Promise(() => {});
  assert.equal(await readOrTick(hung, 1, (ms) => new Promise((r) => setTimeout(r, ms))), null);
});

test('megabytes are shown to one decimal place, and junk is 0', () => {
  assert.equal(megabytes(6_900_000), '6.9 MB');
  assert.equal(megabytes(0), '0.0 MB');
  for (const x of [-1, NaN, Infinity, undefined, null, '5']) assert.equal(megabytes(x), '0.0 MB');
});
