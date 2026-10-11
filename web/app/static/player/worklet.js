// AudioWorklet sink: plays interleaved stereo PCM chunks from the player worker and
// reports how many frames have actually been played (drives LMS elapsed time).
//
// It also applies the volume (linear gains from the page). Applied here, after the queue,
// a change is heard at once; it is ramped over a few milliseconds so it doesn't click.

/** Volume changes are ramped over this long. */
const RAMP_SECS = 0.03;

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
      if (Number.isInteger(msg.epoch)) this.epoch = msg.epoch;
    }
  }

  process(_inputs, outputs) {
    const out = outputs[0];
    const left = out[0];
    const right = out.length > 1 ? out[1] : null; // null: mono output, mix down
    const n = left.length;
    let i = 0;
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
      this.playedSinceReport += take;
      if (this.offset >= buf.length) {
        this.queue.shift();
        this.offset = 0;
      }
    }
    for (; i < n; i++) {
      this.advanceGain();
      left[i] = 0;
      if (right) right[i] = 0;
    }
    // Report in batches while audio flows, and everything once the queue runs dry: the
    // final frames of a playlist must be counted, or the player never reports "played out".
    if (this.worker && (this.playedSinceReport >= 1024 || (this.playedSinceReport > 0 && !this.queue.length))) {
      this.worker.postMessage({ played: this.playedSinceReport, epoch: this.epoch });
      this.playedSinceReport = 0;
    }
    return true;
  }
}

registerProcessor('anywhere-sink', AnywhereSink);
