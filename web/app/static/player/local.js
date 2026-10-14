// Local control of this device's output while the server is out of reach. Pure, unit-tested
// in Node.
//
// The page can hold the output (a pause made here: nothing more is heard, the queued audio
// and the play position are kept) and let it go again. Holding never touches the core or
// what it tells LMS: the output simply stops presenting audio, so the position the core
// reports stands still. Once the server is back and has been told, the hold is let go when
// LMS's own state takes over: at once, after LMS's flush has settled (it restarted the
// track where this device was), or once LMS has paused the player too.

/** Let go of the hold this long after a flush, if no other flush follows. */
export const SETTLE_MS = 800;
/** Never hold longer than this after being told to let go. */
export const RELEASE_DEADLINE_MS = 5000;

const RELEASES = ['now', 'flush', 'paused'];

export class LocalHold {
  constructor() {
    this.held = false;
    // { until, deadline, flushedAt }: letting go once `until` is seen, or at `deadline`.
    this.pending = null;
  }

  /** Hold the output (a pause made on this device). */
  hold() {
    this.held = true;
    this.pending = null;
  }

  /**
   * Let go: 'now', or once LMS has taken over ('flush': its flush has settled; 'paused': it
   * has paused the player), and in any case by the deadline.
   */
  release(until, now) {
    if (!this.held) return;
    if (until === 'now') {
      this.held = false;
      this.pending = null;
      return;
    }
    this.pending = { until, deadline: now + RELEASE_DEADLINE_MS, flushedAt: null };
  }

  /** LMS flushed the player (a new stream, a seek). */
  onFlush(now) {
    if (this.pending?.until === 'flush') this.pending.flushedAt = now;
  }

  /**
   * Every tick, with the core's output state. Returns whether the output is held now.
   */
  update(output, now) {
    const p = this.pending;
    if (this.held && p) {
      const paused = typeof output === 'string' && output.startsWith('Paused');
      const settled = p.until === 'flush' && p.flushedAt !== null && now - p.flushedAt >= SETTLE_MS;
      if (now >= p.deadline || settled || (p.until === 'paused' && paused)) {
        this.held = false;
        this.pending = null;
      }
    }
    return this.held;
  }
}

/**
 * A `local` message from the page, checked: `{ hold: true }`, or `{ release }` with one of
 * the known conditions. Anything else is null (ignored).
 */
export function parseLocal(msg) {
  if (!msg || typeof msg !== 'object') return null;
  if (msg.hold === true && !('release' in msg)) return { hold: true };
  if (!('hold' in msg) && typeof msg.release === 'string' && RELEASES.includes(msg.release)) return { release: msg.release };
  return null;
}

/** The output state as the worker acts on it: 'Held' while held here, else the core's. */
export function effectiveOutput(output, held) {
  return held ? 'Held' : output;
}
