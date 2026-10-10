// AudioWorklet sink: plays interleaved stereo PCM chunks from the player worker and
// reports how many frames have actually been played (drives LMS elapsed time).

class AnywhereSink extends AudioWorkletProcessor {
  constructor() {
    super();
    this.queue = [];
    this.offset = 0;
    this.playedSinceReport = 0;
    this.worker = null;
    this.port.onmessage = (e) => {
      if (e.data.port) {
        this.worker = e.data.port;
        this.worker.onmessage = (m) => this.onWorkerMessage(m.data);
      }
    };
  }

  onWorkerMessage(msg) {
    if (msg.pcm) {
      this.queue.push(msg.pcm);
    } else if (msg.flush) {
      this.queue = [];
      this.offset = 0;
      this.playedSinceReport = 0;
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
        const l = buf[this.offset + 2 * k];
        const r = buf[this.offset + 2 * k + 1];
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
      left[i] = 0;
      if (right) right[i] = 0;
    }
    // Report in batches while audio flows, and everything once the queue runs dry: the
    // final frames of a playlist must be counted, or the player never reports "played out".
    if (this.worker && (this.playedSinceReport >= 1024 || (this.playedSinceReport > 0 && !this.queue.length))) {
      this.worker.postMessage({ played: this.playedSinceReport });
      this.playedSinceReport = 0;
    }
    return true;
  }
}

registerProcessor('anywhere-sink', AnywhereSink);
