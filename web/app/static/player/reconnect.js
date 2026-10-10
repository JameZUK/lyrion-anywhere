// Reconnect policy for the player worker. Pure functions, unit-tested in Node.

// Close codes from the gateway (see gateway/src/session.rs and tunnel.rs).
export const CLOSE = {
  NORMAL: 1000,
  GOING_AWAY: 1001, // LMS closed the connection, or the session expired
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
 */
export function closeAction(code) {
  switch (code) {
    case CLOSE.POLICY:
    case CLOSE.REPLACED:
      return 'stop';
    case CLOSE.NO_SESSION:
    case CLOSE.GOING_AWAY:
      return 'new-session';
    default:
      return 'resume';
  }
}

/** Exponential backoff with jitter: ~0.5 s, 1 s, 2 s, 4 s, then every ~5 s. */
export function backoffMs(attempt, random = Math.random) {
  const base = Math.min(500 * 2 ** attempt, 5000);
  return Math.round(base * (0.8 + 0.4 * random()));
}

/**
 * After a failed stream fetch with HTTP `status` (or 0 for a network error):
 * 'retry' the download from the current offset, or 'fail' the track.
 */
export function streamFailureAction(status) {
  if (status === 0 || status >= 500) return 'retry';
  return 'fail'; // 401 revoked, 404 gone, 410 outside replay window, 416 bad offset
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
  return { wasm, WasmPlayer: mod.WasmPlayer, WasmSegmenter: mod.WasmSegmenter };
}
