// What is audible, for the player worker in Web Audio mode. Pure; unit-tested in Node.
//
// The AudioWorklet reports the audio it renders as runs: `[start, frames, skipped]` means
// `frames` frames were rendered from context frame `start` onwards, and `skipped` frames
// were dropped just before them (a sync skip). Rendering runs ahead of what is heard, by the
// output's latency: tens of milliseconds, or hundreds over Bluetooth.
//
// The page tells the worker which context time is being heard when, from
// `AudioContext.getOutputTimestamp()` (or, where that is missing, from `currentTime` less
// `outputLatency` and `baseLatency`). Times are absolute milliseconds
// (`performance.timeOrigin + performance.now()`), because the page and the worker each have
// their own `performance.now()`.
//
// From those two, `take()` says how many frames have become audible by a given moment.
// That is what the core reports to LMS, so that a synced group hears the same thing at the
// same time. `frameFor()` maps a timed start back to the context frame that will be heard
// at that moment.

/** A clock older than this is not used (the page stopped updating it). */
export const CLOCK_MAX_AGE_MS = 5000;
/** Runs kept at most (a worklet that floods us is cut short, never grows memory). */
export const MAX_RUNS = 4096;
/** Largest plausible run (frames): ten seconds at 384 kHz. */
const MAX_RUN_FRAMES = 3_840_000;

const isCount = (n, max) => Number.isInteger(n) && n >= 0 && n <= max;

export class Presentation {
  #runs = [];
  #clock = null;

  /** Context time `ctx` (seconds) is heard at absolute time `at` (ms), at `rate` Hz. */
  setClock(ctx, at, rate) {
    if (![ctx, at, rate].every(Number.isFinite) || ctx < 0 || at <= 0 || rate <= 0 || rate > 1e6) return false;
    this.#clock = { ctx, at, rate };
    return true;
  }

  /** A new AudioContext (its frames start again at 0): forget the clock and the runs. */
  reset() {
    this.#runs = [];
    this.#clock = null;
  }

  /** A flush: what was rendered belongs to the old position. The clock still holds. */
  dropRuns() {
    this.#runs = [];
  }

  /** Add runs from a worklet report. Malformed entries are ignored. */
  add(runs) {
    if (!Array.isArray(runs)) return;
    for (const r of runs) {
      if (!Array.isArray(r) || r.length !== 3) continue;
      const [start, frames, skipped] = r;
      if (!isCount(start, Number.MAX_SAFE_INTEGER) || !isCount(frames, MAX_RUN_FRAMES) || !isCount(skipped, MAX_RUN_FRAMES)) continue;
      if (frames === 0 && skipped === 0) continue;
      this.#runs.push([start, frames, skipped]);
    }
    if (this.#runs.length > MAX_RUNS) this.#runs.splice(0, this.#runs.length - MAX_RUNS);
  }

  /** Whether the page's clock is usable at absolute time `now`. */
  hasClock(now) {
    return !!this.#clock && now - this.#clock.at <= CLOCK_MAX_AGE_MS && now - this.#clock.at >= -1000;
  }

  /** The context frame heard at absolute time `now`; Infinity (all of it) without a clock. */
  heardFrame(now) {
    if (!this.hasClock(now)) return Infinity;
    const { ctx, at, rate } = this.#clock;
    return Math.floor((ctx + (now - at) / 1000) * rate);
  }

  /** The context frame that will be heard at absolute time `target`, or null without a clock. */
  frameFor(target, now = target) {
    if (!this.hasClock(now) || !Number.isFinite(target)) return null;
    const { ctx, at, rate } = this.#clock;
    return Math.max(0, Math.round((ctx + (target - at) / 1000) * rate));
  }

  /** Frames that have become audible by absolute time `now` (each is counted once). */
  take(now) {
    const heard = this.heardFrame(now);
    let total = 0;
    while (this.#runs.length) {
      const run = this.#runs[0];
      const [start, frames, skipped] = run;
      if (heard < start) break;
      total += skipped;
      const done = Math.min(frames, heard - start);
      total += done;
      if (done >= frames) {
        this.#runs.shift();
        continue;
      }
      // Part of this run is heard: keep the rest.
      run[0] = start + done;
      run[1] = frames - done;
      run[2] = 0;
      break;
    }
    return total;
  }

  /** Frames rendered but not heard yet. */
  pending() {
    return this.#runs.reduce((n, r) => n + r[1] + r[2], 0);
  }
}
