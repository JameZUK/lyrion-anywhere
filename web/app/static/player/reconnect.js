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
};

/**
 * What to do after the player socket closes:
 * - 'stop': don't reconnect (credentials rejected, protocol error, or replaced elsewhere);
 * - 'new-session': the old session is gone; connect afresh (new HELO);
 * - 'resume': reconnect and resume the existing session.
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

/**
 * The first message on the player socket: the bare token for a new session at the original
 * level (as always), otherwise JSON with the session to resume and the level. Gateways
 * ignore a level they don't know, so it is always safe to send here. `keepalive` asks for
 * text keepalives: only for a gateway listing `lms_busy` (older ones never send them).
 */
export function signInMessage(token, resume, quality, keepalive = false) {
  const q = sanitiseQuality(quality);
  if (!resume && q === 'original' && keepalive !== true) return token;
  const m = { token };
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
  if (typeof m.session !== 'string' || typeof m.secret !== 'string') return null;
  return { greeting: m };
}

/**
 * Route one message from the player socket: binary to `frames` (the only kind counted for
 * resume), a greeting to `greeting`, a keepalive nowhere, and anything else to `ignored`.
 * The caller has already noted that the socket is alive.
 */
export function routeMessage(data, to) {
  if (typeof data !== 'string') return to.frames(data);
  const t = classifyText(data);
  if (t === 'keepalive') return;
  if (t) to.greeting(t.greeting);
  else to.ignored();
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
      for (;;) {
        // Back-pressure: stop reading while the core's input buffer is full.
        let parkedAt = -1;
        let parkNoted = false;
        while (io.current() && !player.wants_stream_data()) {
          if (parkedAt < 0) parkedAt = io.now();
          if (!parkNoted && io.now() - parkedAt >= READ_PARKED_NOTE_MS) {
            parkNoted = true;
            let held = 0;
            try {
              held = io.undecoded?.() ?? 0;
            } catch {
              /* for the log only */
            }
            io.log(`stream ${n}: reading paused, core holds ${megabytes(held)} undecoded`);
          }
          await io.sleep(50);
        }
        if (parkNoted && io.current()) io.log(`stream ${n}: reading resumed after ${Math.round((io.now() - parkedAt) / 1000)} s`);
        if (!io.current()) return;
        const { done, value } = await reader.read();
        if (!io.current()) return;
        if (done) {
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
        if (confirming) {
          confirming = false;
          io.log(`stream ${n}: not the end; continued from byte ${offset}`);
        }
        got += value.length;
        tries = 0;
        renewed = false;
        io.mark('bytes');
        player.on_stream_data(value, io.now());
        const have = player.open_stream();
        if (have && have[0] === n && have[1] - lastAck >= io.ackEvery) {
          lastAck = have[1];
          io.ack(n, lastAck);
        }
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
