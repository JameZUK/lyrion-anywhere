// Reconnect policy for the player worker. Pure functions, unit-tested in Node.

// Close codes from the gateway (see gateway/src/session.rs and tunnel.rs).
export const CLOSE = {
  NORMAL: 1000,
  GOING_AWAY: 1001, // the gateway: LMS closed the connection, or the session expired; or the browser
  ABNORMAL: 1006, // network dropped without a close frame
  POLICY: 1008, // unauthorised or protocol violation
  TRY_AGAIN: 1013, // server unavailable, or this client was too slow
  REPLACED: 4000, // another connection for this device took over
  NO_SESSION: 4404, // resume refused: session unknown or expired
  TOO_OLD: 4426, // this client's protocol is older than the gateway's: a stale cached page
};

/**
 * What to do after the player socket closes:
 * - 'stop': don't reconnect (credentials rejected, protocol error, or replaced elsewhere);
 * - 'new-session': the old session is gone; connect afresh (new HELO);
 * - 'resume': reconnect and resume the existing session;
 * - 'too-old': this page is older than the gateway (4426): stop for good, and say to reload.
 *   The same sign-in form can never succeed, so it is never retried.
 *
 * 1001 is resumed too: browsers also use it when they close a socket themselves (a page
 * being suspended), and a new session costs a fresh HELO, which makes LMS restart the
 * track, or stop if it was paused. If the gateway really ended the session, the resume is
 * refused (4404) and only then does a new session start.
 */
export function closeAction(code) {
  switch (code) {
    case CLOSE.POLICY:
    case CLOSE.REPLACED:
      return 'stop';
    case CLOSE.NO_SESSION:
      return 'new-session';
    case CLOSE.TOO_OLD:
      return 'too-old';
    default:
      return 'resume';
  }
}

/**
 * LMS asks every player for its status every 5 s, so a working player socket is never quiet
 * for long. One that has been silent this long has died without closing (a network change):
 * abandon it and resume, well within the gateway's 60 s grace.
 */
export const SOCKET_SILENCE_MS = 20_000;

/** Has the socket been silent too long? (`lastHeard` and `now` in ms.) */
export function socketSilent(lastHeard, now, limit = SOCKET_SILENCE_MS) {
  return now - lastHeard > limit;
}

/** Exponential backoff with jitter: ~0.5 s, 1 s, 2 s, 4 s, then every ~5 s. */
export function backoffMs(attempt, random = Math.random) {
  const base = Math.min(500 * 2 ** attempt, 5000);
  return Math.round(base * (0.8 + 0.4 * random()));
}

/**
 * After a failed stream fetch with HTTP `status` (or 0 for a network error):
 * 'retry' the download from the current offset, 'renew' the access token and then retry
 * (a 401 the first time: it has most likely just expired), or 'fail' the track.
 */
export function streamFailureAction(status, renewed = false) {
  if (status === 0 || status >= 500) return 'retry';
  if (status === 401 && !renewed) return 'renew';
  return 'fail'; // 401 again (revoked), 404 gone, 410 outside replay window, 416 bad offset
}

/**
 * A stream response ended cleanly after bringing `got` bytes: 'end' of the track, or
 * 'confirm' it first with one more request from the offset reached.
 *
 * The gateway ends a response cleanly only at the true end of the track, and with an error
 * otherwise (e.g. after ten minutes without acknowledgements). A proxy on the way can turn that
 * error into a clean end (issue #12, through a Cloudflare Tunnel), which looked like the end
 * of the track: the player skipped to the next one. At the true end, the request from the
 * offset reached is answered at once with nothing; otherwise the rest of the audio follows.
 * Only a response that brought nothing is taken as the end, so a track costs one extra,
 * empty request at most, and asking again always needs progress (no loop).
 */
export function cleanEndAction(got) {
  return got > 0 ? 'confirm' : 'end';
}

/**
 * The request confirming a clean end failed before bringing anything (`action` from
 * {@link streamFailureAction}, after `tries` retries): whether to take the earlier clean end
 * as the track's end after all, as before confirmations, rather than fail the track.
 */
export function confirmGivesUp(action, tries, maxTries) {
  return action === 'fail' || (action === 'retry' && tries >= maxTries);
}

/** Messages for the server, held while disconnected. Bounded: oldest dropped first. */
export class Outbox {
  constructor(limit) {
    this.limit = limit;
    this.items = [];
    this.dropped = 0;
  }

  push(bytes) {
    this.items.push(bytes);
    if (this.items.length > this.limit) {
      this.items.shift();
      this.dropped++;
    }
  }

  drain() {
    const items = this.items;
    this.items = [];
    return items;
  }

  clear() {
    this.items = [];
  }

  get length() {
    return this.items.length;
  }
}

/**
 * Asks the page for an AudioContext at the core's output rate once per change, not on every
 * 20 ms tick: each request makes the page build a new context.
 */
export class RateRequest {
  constructor() {
    this.requested = 0;
  }

  /** Ask for `rate`, given the page's context runs at `ready`? */
  want(rate, ready) {
    if (!rate || rate === ready || rate === this.requested) return false;
    this.requested = rate;
    return true;
  }

  reset() {
    this.requested = 0;
  }
}

/**
 * The gateway refused the access token: wait for the page to send a renewed one, then
 * reconnect once, whichever message brings it.
 */
export class AuthWait {
  constructor() {
    this.waiting = false;
  }

  refused() {
    this.waiting = true;
  }

  /** A new token arrived: reconnect now? (Not while connected, and only once.) */
  token(open) {
    if (!this.waiting || open) return false;
    this.waiting = false;
    return true;
  }
}

/**
 * Load the WebAssembly core, with the app's build version (`?v=`, from the worker's own URL)
 * on both its files. A browser holding an older copy in its cache then fetches the new one
 * instead of pairing stale code with a new app. `importer` is `(url) => import(url)`.
 */
export async function loadCore(workerUrl, importer) {
  const v = new URL(workerUrl).searchParams.get('v');
  const q = v && /^[\w.-]{1,64}$/.test(v) ? `?v=${v}` : '';
  const mod = await importer(new URL(`./pkg/anywhere_wasm.js${q}`, workerUrl).href);
  const wasm = await mod.default({ module_or_path: new URL(`./pkg/anywhere_wasm_bg.wasm${q}`, workerUrl).href });
  // Every class the module exports (a list of names here once left out the visualiser's).
  const classes = Object.fromEntries(Object.entries(mod).filter(([k, c]) => /^Wasm[A-Z]\w*$/.test(k) && typeof c === 'function'));
  return { ...classes, wasm };
}

/**
 * Times a seek (or any flush) to the moment the new audio is heard, so the connection log
 * shows where the time went on a real device. Starts at the flush; each stage is recorded
 * once, in milliseconds after it.
 */
export const SEEK_STAGES = ['stream', 'response', 'bytes', 'started', 'queued', 'audible'];
const SEEK_LABELS = {
  stream: 'new stream',
  response: 'response',
  bytes: 'first bytes',
  started: 'started',
  queued: 'queued',
};
// A seek that hasn't been heard after this long is dropped (e.g. LMS stopped instead).
export const SEEK_GIVE_UP_MS = 30_000;

export class SeekTimer {
  constructor() {
    this.at = -1;
    this.marks = {};
  }

  get active() {
    return this.at >= 0;
  }

  /** A flush at `t` (ms): start timing, replacing any timing still running. */
  start(t) {
    this.at = t;
    this.marks = {};
  }

  /**
   * Stage `name` reached at `t`. Returns the finished timing once the audio is heard,
   * `{ total, marks }` (ms after the flush); otherwise null.
   */
  mark(name, t) {
    if (!this.active || !SEEK_STAGES.includes(name) || Object.hasOwn(this.marks, name)) return null;
    // Audio played before any of the new audio was handed to the output is the old position.
    if (name === 'audible' && !Object.hasOwn(this.marks, 'queued')) return null;
    if (!(t - this.at <= SEEK_GIVE_UP_MS)) {
      this.at = -1;
      return null;
    }
    this.marks[name] = Math.max(0, Math.round(t - this.at));
    if (name !== 'audible') return null;
    const done = { total: this.marks.audible, marks: { ...this.marks } };
    this.at = -1;
    return done;
  }

  cancel() {
    this.at = -1;
  }
}

/** One log line for a finished seek timing. */
export function describeSeek(timing) {
  const parts = SEEK_STAGES.filter((s) => s !== 'audible' && Object.hasOwn(timing.marks, s)).map(
    (s) => `${SEEK_LABELS[s]} +${timing.marks[s]}`,
  );
  return `seek: heard after ${timing.total} ms${parts.length ? ` (${parts.join(', ')})` : ''}`;
}

/**
 * Follows the core's flush count. LMS sends a seek's flush and the new stream back to back
 * (often in one message), so the output state alone can go from playing to waiting without
 * ever reading "stopped": the count is what says the queued audio must go.
 */
export class FlushWatch {
  constructor() {
    this.count = 0;
  }

  /** The core's count now: true if it flushed since the last call. */
  changed(count) {
    if (count === this.count) return false;
    this.count = count;
    return true;
  }

  /** A new core player (counting from zero again). */
  reset() {
    this.count = 0;
  }
}

// --- Streaming quality (docs/plans/streaming-quality.md) --------------------------------

/** The levels a player may ask the gateway for. */
export const QUALITIES = ['original', 'high', 'low'];

/** A level from the page, checked: anything else is 'original'. */
export function sanitiseQuality(q) {
  return QUALITIES.includes(q) ? q : 'original';
}

/** The player protocol this worker speaks (docs/PROTOCOL.md, *Versions*). */
export const PROTOCOL = 2;

/**
 * The first message on the player socket: JSON with the protocol version (required since
 * protocol 2: a gateway closes a form without it with 4426), the session to resume, the
 * level and whether to send text keepalives. `keepalive` asks for them only when told to (a
 * gateway listing `lms_busy`).
 */
export function signInMessage(token, resume, quality, keepalive = false) {
  const q = sanitiseQuality(quality);
  const m = { token, protocol: PROTOCOL };
  if (resume) m.resume = { session: resume.session, secret: resume.secret, received: resume.received };
  if (q !== 'original') m.quality = q;
  if (keepalive === true) m.keepalive = true;
  return JSON.stringify(m);
}

// The greeting's fields: a text message carrying any of them is never a keepalive.
const GREETING_FIELDS = ['session', 'secret', 'mac', 'name', 'resumed', 'grace'];

/**
 * What a text message on the player socket is (docs/PROTOCOL.md):
 * - 'keepalive': exactly `{"keepalive":true}` (sent every 5 s to a client that asked for
 *   it, so a socket left waiting on a busy LMS isn't taken for dead). It only shows the
 *   socket is alive: it is not a greeting and carries no sequence number;
 * - { stream }: the stream message (protocol 2), checked by `parseStreamMessage`;
 * - 'bad-stream': a message with a `stream` field that fails those checks;
 * - { greeting }: the gateway's greeting (a session and secret, as strings);
 * - null: anything else (not JSON, oversized, or a mixture of the two), which is ignored.
 */
export function classifyText(text) {
  if (typeof text !== 'string' || text.length > 4096) return null;
  let m;
  try {
    m = JSON.parse(text);
  } catch {
    return null;
  }
  if (!m || typeof m !== 'object' || Array.isArray(m)) return null;
  if ('keepalive' in m) {
    if (m.keepalive !== true || GREETING_FIELDS.some((f) => f in m)) return null;
    return 'keepalive';
  }
  if ('stream' in m) {
    const stream = text.length <= STREAM_MESSAGE_MAX ? parseStreamMessage(m) : null;
    return stream ? { stream } : 'bad-stream';
  }
  if (typeof m.session !== 'string' || typeof m.secret !== 'string') return null;
  return { greeting: m };
}

/**
 * Route one message from the player socket: binary to `frames` (the only kind counted for
 * resume), a greeting to `greeting`, a stream message to `stream`, a malformed stream
 * message to `badStream`, a keepalive nowhere, and anything else to `ignored`. The caller
 * has already noted that the socket is alive.
 */
export function routeMessage(data, to) {
  if (typeof data !== 'string') return to.frames(data);
  const t = classifyText(data);
  if (t === 'keepalive') return;
  if (t === 'bad-stream') (to.badStream ?? to.ignored)();
  else if (t?.stream) to.stream?.(t.stream);
  else if (t) to.greeting(t.greeting);
  else to.ignored();
}

// --- The stream message (protocol 2; docs/PROTOCOL.md, *Messages after the greeting*) ---------

/** The gateway sends at most this much in one stream message. */
export const STREAM_MESSAGE_MAX = 512;
/** Codec tokens are short lower-case words (`flac`, `pcm`, `mp3`, `ogg`, `aac`, `alac`, `unknown`). */
const CODEC_TOKEN = /^[a-z][a-z0-9]{0,11}$/;
const BAD = Symbol('bad');
const whole = (v, lo, hi) => Number.isSafeInteger(v) && v >= lo && v <= hi;
/** An optional whole number in range: null when absent or null, BAD when anything else. */
const optWhole = (v, lo, hi) => (v === undefined || v === null ? null : whole(v, lo, hi) ? v : BAD);
const plain = (v) => !!v && typeof v === 'object' && !Array.isArray(v);

function parseServed(v) {
  if (!plain(v)) return null;
  const codec = v.codec === undefined || v.codec === null ? null : typeof v.codec === 'string' && CODEC_TOKEN.test(v.codec) ? v.codec : BAD;
  const rate = optWhole(v.rate, 1000, 10_000_000);
  const bits = optWhole(v.bits, 1, 64);
  const channels = optWhole(v.channels, 1, 64);
  if ([codec, rate, bits, channels].includes(BAD)) return null;
  const out = { codec, rate, bits, channels };
  if (v.gain_db !== undefined && v.gain_db !== null) {
    if (typeof v.gain_db !== 'number' || !Number.isFinite(v.gain_db) || Math.abs(v.gain_db) > 100) return null;
    out.gain_db = v.gain_db;
  }
  return out;
}

function parseSent(v) {
  if (!plain(v)) return null;
  if (v.mode === 'passthrough') return { mode: 'passthrough' };
  if (v.mode === 'opus' && whole(v.kbps, 6, 1000) && (v.level === 'high' || v.level === 'low')) return { mode: 'opus', kbps: v.kbps, level: v.level };
  return null;
}

/**
 * The stream message `{"stream":{"seq","served","sent"}}`, checked strictly: `seq` a positive
 * whole number, `served.codec` a short lower-case token (or null), every number finite and
 * in sane bounds, `sent` pass-through or Opus with its bit rate and a known level. Returns
 * a fresh object holding only those fields, or null for anything else. It crosses to the
 * page and ends up on screen, so nothing else is passed on.
 */
export function parseStreamMessage(m) {
  if (!plain(m) || Object.keys(m).length !== 1 || !plain(m.stream)) return null;
  const s = m.stream;
  if (!whole(s.seq, 1, 2 ** 31)) return null;
  const served = parseServed(s.served);
  const sent = parseSent(s.sent);
  if (!served || !sent) return null;
  return { seq: s.seq, served, sent };
}

/** A server → client Slimproto frame that starts a stream: `strm` with command `s`. */
export function isStreamStart(bytes) {
  const b = bytes instanceof Uint8Array ? bytes : bytes instanceof ArrayBuffer ? new Uint8Array(bytes) : null;
  // Two length bytes, the opcode, then the command.
  return !!b && b.length >= 7 && b[2] === 0x73 && b[3] === 0x74 && b[4] === 0x72 && b[5] === 0x6d && b[6] === 0x73;
}

/**
 * What the gateway said about each stream of this session, tied to the streams by `seq`: the
 * count of `strm s` frames, from 1, which both sides keep the same way. Frames are counted by
 * their place in the session (the resume counter), so a frame seen twice is never counted
 * twice. The last few messages are kept (a later one for the same `seq`, as re-sent after a
 * resume, replaces it).
 */
export class StreamInfo {
  static KEEP = 8;
  #starts = 0;
  #lastFrame = 0;
  #bySeq = new Map();

  /** A new session: nothing is known, and counting starts again. */
  reset() {
    this.#starts = 0;
    this.#lastFrame = 0;
    this.#bySeq.clear();
  }

  /** Binary message number `n` of the session (from 1); true when it started a stream. */
  frame(n, bytes) {
    if (!Number.isSafeInteger(n) || n <= this.#lastFrame) return false;
    this.#lastFrame = n;
    if (!isStreamStart(bytes)) return false;
    this.#starts++;
    return true;
  }

  /** How many `strm s` frames this session has seen. */
  get starts() {
    return this.#starts;
  }

  /** Keep a checked stream message (see parseStreamMessage). */
  add(info) {
    this.#bySeq.delete(info.seq);
    this.#bySeq.set(info.seq, info);
    while (this.#bySeq.size > StreamInfo.KEEP) {
      const oldest = Math.min(...this.#bySeq.keys());
      this.#bySeq.delete(oldest);
    }
  }

  /** The message for stream `seq`, or null. */
  of(seq) {
    return this.#bySeq.get(seq) ?? null;
  }
}

/** A change of level mid-session: only for a gateway listing `stream_quality` (older ones end the session on it). */
export function qualityMessage(quality) {
  return JSON.stringify({ quality: sanitiseQuality(quality) });
}

/**
 * The address of stream `n` of `session`, from byte `offset`. The session id is the gateway's,
 * but it is encoded all the same, so it can only ever be one path segment; the stream number
 * and offset are whole numbers or left out.
 */
export function streamPath(session, n, offset) {
  const num = Number.isSafeInteger(n) && n >= 0 ? n : 0;
  const q = Number.isSafeInteger(offset) && offset > 0 ? `?offset=${offset}` : '';
  return `/v1/stream/${encodeURIComponent(String(session))}/${num}${q}`;
}

/**
 * Reading a stream parked this long (ms) because the core's buffer is full is logged once
 * (issue #12: it shows the output, not the network, is what holds the stream up).
 */
export const READ_PARKED_NOTE_MS = 10_000;
/**
 * A response that brings nothing for this long while the core is hungry (nothing read waits
 * for it) is given up and the download resumed from its offset. Safari has been seen to stop delivering a response
 * body it already holds (issue #12: a 24/96 FLAC on an iPad stopped after 3.6 MB with
 * 3.5 MB received and unread); the gateway replays from the offset, so nothing is lost.
 */
export const READ_STALL_MS = 15_000;

/** The core is fed, and a quiet read checked, this often (ms) while a response is open. */
export const FEED_TICK_MS = 50;
/**
 * Bytes read but not yet fed beyond which reading pauses. The gateway sends at most its
 * replay window (4 MiB by default) past the last acknowledgement, and only fed bytes are
 * acknowledged, so this is a backstop, not the usual bound.
 */
export const PENDING_MAX = 16 * 1024 * 1024;

/**
 * `inFlight` (a read in progress), or null once `tickMs` has passed with the read still
 * pending (`sleep(ms)` resolves after ms). The read is left in flight for the next tick.
 */
export function readOrTick(inFlight, tickMs, sleep) {
  return Promise.race([inFlight, sleep(tickMs).then(() => null)]);
}

/** Bytes as megabytes to one decimal place. */
export function megabytes(bytes) {
  return `${(Number.isFinite(bytes) && bytes > 0 ? bytes / 1e6 : 0).toFixed(1)} MB`;
}

/**
 * Download stream `n` into the core (docs/PROTOCOL.md "Streams"). A download that breaks is
 * fetched again from the byte offset the core has reached (`?offset=`), never taken as the
 * end of the track; a 401 renews the token once; a clean end is confirmed first (see
 * {@link cleanEndAction}). Bytes are acknowledged every `io.ackEvery`, so the gateway keeps
 * reading from LMS.
 *
 * `io` is the worker's side: `player` (the core), `current()` (still the stream wanted),
 * `fetch(path, init)`, `session()`, `token()`, `setController(c)`, `sleep(ms)`, `now()`,
 * `log(msg)`, `mark(stage)` (seek timing), `ack(n, bytes)`, `runActions()`, `renewToken()`,
 * `received()` (the stream has come to its end), `maxTries` and `ackEvery`; optionally
 * `undecoded()`, the bytes the core holds not yet decoded (for the log only).
 */
export async function downloadStream(io, n) {
  const { player } = io;
  let headersSeen = false;
  let lastAck = 0;
  let renewed = false;
  // The last response ended cleanly: this request checks that it was the track's end.
  let confirming = false;
  const ended = () => {
    io.received();
    player.on_stream_end(io.now());
    io.runActions();
  };
  for (let tries = 0; ; tries++) {
    // Resume from what the core has already received for this stream.
    const os = player.open_stream();
    const offset = os && os[0] === n ? os[1] : 0;
    const controller = new AbortController();
    io.setController(controller);
    let status = 0;
    let got = 0; // bytes this response brought
    try {
      const resp = await io.fetch(streamPath(io.session(), n, offset), {
        headers: { Authorization: `Bearer ${io.token()}` },
        signal: controller.signal,
        cache: 'no-store',
      });
      if (!io.current()) return;
      status = resp.status;
      if (!resp.ok) throw new Error(`stream HTTP ${resp.status}`);
      // From here on, a failure is the connection breaking mid-download: retryable.
      status = 0;
      if (!headersSeen) {
        headersSeen = true;
        io.mark('response');
        const ct = resp.headers.get('content-type') || 'application/octet-stream';
        player.on_stream_headers(new TextEncoder().encode(`HTTP/1.0 200 OK\r\nContent-Type: ${ct}\r\n\r\n`), io.now());
      } else if (!confirming) {
        io.log(`stream ${n} resumed at byte ${offset}`);
      }
      const reader = resp.body.getReader();
      // The response is read as it arrives, into `pending`, whatever the core wants just
      // now: Safari stops delivering a body whose consumer paused (issue #12: a 24/96 FLAC
      // on an iPad stopped after every pause, with megabytes received and unread). The
      // gateway's window beyond the last acknowledgement bounds what waits here, since only
      // what the core has taken is acknowledged. A read stays in flight across feed ticks.
      const pending = [];
      let pendingBytes = 0;
      let inFlight = null;
      let readSince = io.now();
      let feedPausedAt = -1;
      let feedNoted = false;
      const stallMs = io.readStallMs ?? READ_STALL_MS;
      const feed = () => {
        // Feed what the core takes now; say so once a full core has held it up for a while.
        let fed = false;
        while (pending.length && io.current() && player.wants_stream_data()) {
          const chunk = pending.shift();
          pendingBytes -= chunk.length;
          if (confirming) {
            confirming = false;
            io.log(`stream ${n}: not the end; continued from byte ${offset}`);
          }
          player.on_stream_data(chunk, io.now());
          fed = true;
          const have = player.open_stream();
          if (have && have[0] === n && have[1] - lastAck >= io.ackEvery) {
            lastAck = have[1];
            io.ack(n, lastAck);
            io.logAs?.(`stream ${n}: acknowledged ${megabytes(lastAck)}`, 'stream:ack');
          } else if (!have || have[0] !== n) {
            // Only acknowledged bytes let the gateway send more: a stream fed but never
            // acknowledged stalls after one window (issue #12).
            io.logAs?.(`stream ${n}: fed but not acknowledged (the core is fetching ${have ? `stream ${have[0]}` : 'nothing'})`, 'stream:noack');
          }
        }
        if (pending.length && io.current()) {
          if (feedPausedAt < 0) feedPausedAt = io.now();
          if (!feedNoted && io.now() - feedPausedAt >= READ_PARKED_NOTE_MS) {
            feedNoted = true;
            let held = 0;
            try {
              held = io.undecoded?.() ?? 0;
            } catch {
              /* for the log only */
            }
            io.log(`stream ${n}: feeding paused, core holds ${megabytes(held)} undecoded (${megabytes(pendingBytes)} more read)`);
          }
        } else if (fed) {
          if (feedNoted) io.log(`stream ${n}: feeding resumed after ${Math.round((io.now() - feedPausedAt) / 1000)} s`);
          feedPausedAt = -1;
          feedNoted = false;
        }
      };
      for (;;) {
        feed();
        if (!io.current()) return;
        if (pendingBytes >= PENDING_MAX) {
          // Far more than the gateway's window waits unfed (not expected): read no further.
          await io.sleep(FEED_TICK_MS);
          continue;
        }
        if (!inFlight) {
          inFlight = reader.read();
          readSince = io.now();
        }
        const read = await readOrTick(inFlight, FEED_TICK_MS, io.sleep);
        if (!io.current()) return;
        if (read === null) {
          // A tick with nothing read yet. Stuck only if the core has gone hungry meanwhile:
          // a quiet response is otherwise the gateway waiting for an acknowledgement, which
          // comes as the core takes what is pending (see READ_STALL_MS).
          if (stallMs > 0 && !pending.length && player.wants_stream_data() && io.now() - readSince >= stallMs) {
            const at = player.open_stream();
            io.log(`stream ${n}: no data for ${Math.round(stallMs / 1000)} s at byte ${at && at[0] === n ? at[1] : offset + got}; resuming`);
            controller.abort();
            throw new Error('read stalled');
          }
          continue;
        }
        inFlight = null;
        const { done, value } = read;
        if (done) {
          // Everything read is fed before the end counts.
          while (pending.length) {
            feed();
            if (!io.current()) return;
            if (pending.length) await io.sleep(FEED_TICK_MS);
          }
          if (cleanEndAction(got) === 'confirm') {
            // Maybe not the end: ask again from here (see cleanEndAction).
            confirming = true;
            tries = -1; // the next request is the first of its own
            const at = player.open_stream();
            io.log(`stream ${n}: clean end at byte ${at && at[0] === n ? at[1] : offset + got}, confirming`);
            break;
          }
          if (confirming) io.log(`stream ${n}: end confirmed at byte ${offset}`);
          ended();
          return;
        }
        if (!value || !value.length) continue;
        got += value.length;
        tries = 0;
        renewed = false;
        io.mark('bytes');
        pending.push(value);
        pendingBytes += value.length;
      }
    } catch (err) {
      if (err?.name === 'AbortError' || !io.current()) return;
      const action = streamFailureAction(status, renewed);
      if (confirming && got === 0 && confirmGivesUp(action, tries, io.maxTries)) {
        // Can't tell: take the clean end as the track's end, as before (see cleanEndAction).
        io.log(`stream ${n}: could not confirm its end at byte ${offset} (${err?.message}); taking it as the end`);
        ended();
        return;
      }
      if (action === 'renew') {
        // Most likely the access token expired (a long pause, or a page in the background
        // whose renewal timer was held back): get a new one, then carry on.
        renewed = true;
        io.log(`stream ${n}: access token refused; renewing`);
        await io.renewToken();
        if (!io.current()) return;
        continue;
      }
      if (action === 'fail' || tries >= io.maxTries) {
        io.log(`stream ${n} failed after ${tries} retries at byte ${offset}: ${err?.message} (HTTP ${status || 'none'})`);
        player.on_stream_error(io.now());
        io.runActions();
        return;
      }
      const at = player.open_stream();
      io.log(`stream ${n} interrupted at byte ${at && at[0] === n ? at[1] : offset} (${err?.message}); retrying`);
      await io.sleep(backoffMs(tries));
      if (!io.current()) return;
    }
  }
}
