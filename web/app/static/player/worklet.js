// AudioWorklet sink: plays interleaved stereo PCM chunks from the player worker and
// reports what it has rendered, as runs of context frames, so the worker can tell (with the
// page's output timestamps) what is audible: that drives LMS's elapsed time.
//
// It also applies the volume (linear gains from the page). Applied here, after the queue,
// a change is heard at once; it is ramped over a few milliseconds so it doesn't click.
//
// Sync corrections from LMS are applied here too, at the output, to the frame:
// - `hold`: silence, consuming nothing (paused, or loaded and waiting to start);
// - `startAt`: while held, start the queued audio exactly at that context frame (a timed
//   start). If it is already past, drop the audio that would have played since, and start;
// - `silence`: insert N frames of silence (LMS pausing a player that is ahead);
// - `skip`: drop the next N frames of audio (LMS moving a player that is behind forward).
//
// And `trim`: drop the last N frames queued (LMS dropped the tracks after the one playing;
// `strm f`). Only for the current epoch: after a flush, the queue is already new audio.

/** Volume changes are ramped over this long. */
const RAMP_SECS = 0.03;
/** A late timed start drops at most this much audio (seconds) to catch up. */
const MAX_CATCH_UP_SECS = 1;
/** A timed start further ahead than this (seconds) is refused. */
const MAX_START_AHEAD_SECS = 60;
/** Largest correction accepted (frames): ten seconds at 384 kHz. */
const MAX_CORRECTION = 3_840_000;

const frameCount = (n) => (Number.isInteger(n) && n >= 0 && n <= MAX_CORRECTION ? n : null);

/** A gain from a message: finite, 0..1 (never amplifies), or null if unusable. */
function cleanGain(g) {
  return typeof g === 'number' && Number.isFinite(g) ? Math.min(1, Math.max(0, g)) : null;
}

class AnywhereSink extends AudioWorkletProcessor {
  constructor() {
    super();
    this.queue = [];
    this.offset = 0;
    this.playedSinceReport = 0;
    this.epoch = 0;
    this.worker = null;
    // Current and target gains, and the per-sample step towards the target.
    this.gain = [1, 1];
    this.target = [1, 1];
    this.step = [0, 0];
    this.rate = typeof sampleRate === 'number' && sampleRate > 0 ? sampleRate : 48000;
    // Our own count of frames rendered, where the global `currentFrame` is missing.
    this.frame = 0;
    this.hold = false;
    this.startAt = null;
    this.silence = 0;
    this.skip = 0;
    // Rendered audio not yet reported: [start frame, frames, frames skipped before them].
    this.runs = [];
    this.port.onmessage = (e) => {
      const d = e.data;
      if (!d || typeof d !== 'object') return;
      if (d.port) {
        this.worker = d.port;
        this.worker.onmessage = (m) => this.onWorkerMessage(m.data);
      }
      if (Array.isArray(d.gain)) this.setGain(d.gain, d.instant === true);
    };
  }

  setGain(g, instant) {
    const l = cleanGain(g[0]);
    const r = cleanGain(g.length > 1 ? g[1] : g[0]);
    if (l === null || r === null) return;
    this.target = [l, r];
    if (instant) {
      this.gain = [l, r];
      this.step = [0, 0];
      return;
    }
    const n = Math.max(1, Math.round(this.rate * RAMP_SECS));
    this.step = [(l - this.gain[0]) / n, (r - this.gain[1]) / n];
  }

  /** Move one sample towards the target gain. */
  advanceGain() {
    for (let c = 0; c < 2; c++) {
      const s = this.step[c];
      if (s === 0) continue;
      const g = this.gain[c] + s;
      if ((s > 0 && g >= this.target[c]) || (s < 0 && g <= this.target[c])) {
        this.gain[c] = this.target[c];
        this.step[c] = 0;
      } else {
        this.gain[c] = g;
      }
    }
  }

  onWorkerMessage(msg) {
    if (!msg || typeof msg !== 'object') return;
    if (msg.pcm) {
      this.queue.push(msg.pcm);
    } else if (msg.flush) {
      this.queue = [];
      this.offset = 0;
      this.playedSinceReport = 0;
      this.runs = [];
      this.startAt = null;
      this.silence = 0;
      this.skip = 0;
      if (Number.isInteger(msg.epoch)) this.epoch = msg.epoch;
    } else if (typeof msg.hold === 'boolean') {
      this.hold = msg.hold;
      if (!msg.hold) this.startAt = null;
    } else if (msg.startAt !== undefined) {
      // A start more than a minute ahead is not a real one: it would hold the output silent.
      const ok = Number.isInteger(msg.startAt) && msg.startAt >= 0 && msg.startAt <= this.frame + this.rate * MAX_START_AHEAD_SECS;
      const f = ok ? msg.startAt : null;
      if (f !== null) {
        this.startAt = f;
        this.hold = true;
      }
    } else if (msg.silence !== undefined) {
      const n = frameCount(msg.silence);
      if (n !== null) this.silence = Math.min(MAX_CORRECTION, this.silence + n);
    } else if (msg.skip !== undefined) {
      const n = frameCount(msg.skip);
      if (n !== null) this.skip = Math.min(MAX_CORRECTION, this.skip + n);
    } else if (msg.trim !== undefined) {
      const n = frameCount(msg.trim);
      if (n !== null && msg.epoch === this.epoch) this.dropLast(n);
    }
  }

  /** Drop up to `n` frames from the end of the queue; returns how many were dropped. */
  dropLast(n) {
    let dropped = 0;
    while (dropped < n && this.queue.length) {
      const last = this.queue.length - 1;
      const buf = this.queue[last];
      const start = last === 0 ? this.offset : 0;
      const have = (buf.length - start) / 2;
      const take = Math.min(have, n - dropped);
      dropped += take;
      if (take === have) {
        this.queue.pop();
        if (!this.queue.length) this.offset = 0;
      } else {
        this.queue[last] = buf.subarray(0, buf.length - take * 2);
      }
    }
    return dropped;
  }

  /** Drop up to `n` queued frames; returns how many were dropped. */
  drop(n) {
    let dropped = 0;
    while (dropped < n && this.queue.length) {
      const buf = this.queue[0];
      const take = Math.min((buf.length - this.offset) / 2, n - dropped);
      dropped += take;
      this.offset += take * 2;
      if (this.offset >= buf.length) {
        this.queue.shift();
        this.offset = 0;
      }
    }
    return dropped;
  }

  /** Remember `frames` frames rendered from context frame `start`, after `skipped` dropped. */
  record(start, frames, skipped) {
    if (!frames && !skipped) return;
    const last = this.runs[this.runs.length - 1];
    if (last && !skipped && last[0] + last[1] === start) last[1] += frames;
    else this.runs.push([start, frames, skipped]);
    this.playedSinceReport += frames + skipped;
  }

  /** Fill output frames [from, to) with silence (moving the volume ramp along). */
  silent(left, right, from, to) {
    for (let i = from; i < to; i++) {
      this.advanceGain();
      left[i] = 0;
      if (right) right[i] = 0;
    }
  }

  process(_inputs, outputs) {
    const out = outputs[0];
    const left = out[0];
    const right = out.length > 1 ? out[1] : null; // null: mono output, mix down
    const n = left.length;
    const base = typeof currentFrame === 'number' ? currentFrame : this.frame;
    this.frame = base + n;
    let i = 0;
    let skipped = 0;
    if (this.hold) {
      const at = this.startAt;
      if (at === null || at >= base + n) {
        this.silent(left, right, 0, n);
        this.report();
        return true;
      }
      if (at >= base) {
        i = at - base;
      } else {
        // Too late for the requested frame: drop what would already have played.
        skipped += this.drop(Math.min(base - at, Math.round(this.rate * MAX_CATCH_UP_SECS)));
      }
      this.hold = false;
      this.startAt = null;
    }
    if (this.silence > 0) {
      const s = Math.min(this.silence, n - i);
      this.silence -= s;
      i += s;
    }
    if (this.skip > 0) {
      const d = this.drop(this.skip);
      this.skip -= d;
      skipped += d;
    }
    this.silent(left, right, 0, i);
    const start = i;
    while (i < n && this.queue.length) {
      const buf = this.queue[0];
      const take = Math.min((buf.length - this.offset) / 2, n - i);
      for (let k = 0; k < take; k++) {
        this.advanceGain();
        const l = buf[this.offset + 2 * k] * this.gain[0];
        const r = buf[this.offset + 2 * k + 1] * this.gain[1];
        if (right) {
          left[i + k] = l;
          right[i + k] = r;
        } else {
          left[i + k] = (l + r) / 2;
        }
      }
      i += take;
      this.offset += take * 2;
      if (this.offset >= buf.length) {
        this.queue.shift();
        this.offset = 0;
      }
    }
    this.record(base + start, i - start, skipped);
    this.silent(left, right, i, n);
    this.report();
    return true;
  }

  /**
   * Report in batches while audio flows, and everything once the queue runs dry: the final
   * frames of a playlist must be counted, or the player never reports "played out".
   */
  report() {
    if (this.worker && (this.playedSinceReport >= 1024 || (this.playedSinceReport > 0 && !this.queue.length))) {
      this.worker.postMessage({ played: this.playedSinceReport, epoch: this.epoch, runs: this.runs });
      this.playedSinceReport = 0;
      this.runs = [];
    }
  }
}

registerProcessor('anywhere-sink', AnywhereSink);
