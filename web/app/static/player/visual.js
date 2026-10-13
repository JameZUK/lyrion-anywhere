// The visualiser's tap in the player worker (docs/plans/wave2-design.md §4.1).
//
// The worker pulls PCM ahead of what is heard (0.3 s for Web Audio, up to 8 s for the media
// element), so frames are analysed as the PCM is pulled, tagged with where they fall in the
// output, and handed to the page only once the output has played that far. Nothing runs
// unless the page has asked for frames (the visualiser is on and showing).

/** Values per frame: 32 bands, 128 scope points, RMS L/R, peak L/R, bass (see wasm/). */
export const FRAME_LEN = 32 + 128 + 5;
/** Frames waiting for the output to reach them (8 s at 30 a second, with room to spare). */
export const MAX_QUEUED = 512;
/** At most 30 frames a second to the page. */
export const MAX_FPS = 30;
export const MIN_GAP_MS = 1000 / MAX_FPS - 3;
/** Audio kept while off in media mode: what the element may still have to play. */
export const RETAIN_SECS = 10;

export class VisualTap {
  /** `make(rate, channels)` builds the analyser (the core's WasmVisualiser). */
  constructor(make) {
    this.make = make;
    this.on = false;
    this.vis = null;
    this.rate = 0;
    this.queue = [];
    this.lastSent = -Infinity;
    /** Media mode, while off: the PCM already sent ahead (see `feed`'s `retain`). */
    this.ahead = [];
    this.aheadFrames = 0;
  }

  /** The page wants frames (true) or not (false): off drops everything, analyser included. */
  setOn(on) {
    const want = on === true;
    if (want === this.on) return;
    this.on = want;
    if (!want) {
      this.#drop();
      return;
    }
    // The media element plays from seconds of audio already sent ahead: analyse that now,
    // or nothing would be due until the newly pulled audio is heard (up to 8 s later).
    const kept = this.ahead;
    this.ahead = [];
    this.aheadFrames = 0;
    for (const k of kept) this.feed(k.pcm, k.rate, k.at);
  }

  #drop() {
    this.queue.length = 0;
    if (this.vis) {
      try {
        this.vis.free?.();
      } catch {
        /* already freed */
      }
    }
    this.vis = null;
    this.rate = 0;
  }

  /** The output was flushed (a seek, a stop, a new session): forget what was queued. */
  flush() {
    this.queue.length = 0;
    this.ahead = [];
    this.aheadFrames = 0;
    try {
      this.vis?.reset();
    } catch {
      this.#drop();
    }
  }

  /**
   * Interleaved stereo PCM at `rate`, about to go to the output; `at` is how many frames
   * were pulled before it (the output's position when its first frame is heard).
   * `retain`: while off, keep it (by reference: the caller never reuses it) so turning on
   * can start from what is about to be heard; at most RETAIN_SECS of it.
   */
  feed(pcm, rate, at, retain = false) {
    if (!(rate > 0) || !pcm || !(pcm.length >= 2) || !Number.isFinite(at)) return;
    if (!this.on) {
      if (retain) {
        this.ahead.push({ pcm, rate, at });
        this.aheadFrames += pcm.length / 2;
        while (this.ahead.length > 1 && this.aheadFrames - this.ahead[0].pcm.length / 2 > rate * RETAIN_SECS) {
          this.aheadFrames -= this.ahead.shift().pcm.length / 2;
        }
      }
      return;
    }
    if (!this.vis || this.rate !== rate) {
      this.#drop();
      try {
        this.vis = this.make(rate, 2);
      } catch {
        this.vis = null;
        return;
      }
      this.rate = rate;
    }
    // In slices of one frame's worth of audio, so a long chunk still gives every frame.
    const hop = Math.max(1, Math.ceil(rate / MAX_FPS)) * 2;
    const len = pcm.length - (pcm.length % 2);
    for (let i = 0; i < len; i += hop) {
      const end = Math.min(len, i + hop);
      this.vis.push(pcm.subarray(i, end));
      const f = this.vis.take();
      if (f && f.length === FRAME_LEN) {
        this.queue.push({ at: at + end / 2, f });
        if (this.queue.length > MAX_QUEUED) this.queue.shift();
      }
    }
  }

  /**
   * The newest frame the output has now played (`played` frames since the last flush), at
   * most one per MIN_GAP_MS, or null. Older frames are dropped.
   */
  due(played, nowMs) {
    if (!this.on || !this.queue.length || !Number.isFinite(played)) return null;
    if (nowMs - this.lastSent < MIN_GAP_MS) return null;
    let k = -1;
    while (k + 1 < this.queue.length && this.queue[k + 1].at <= played) k++;
    if (k < 0) return null;
    const { f } = this.queue[k];
    this.queue.splice(0, k + 1);
    this.lastSent = nowMs;
    return f;
  }
}
