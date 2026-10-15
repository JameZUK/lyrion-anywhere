// LMS's sync corrections in media-element mode, for the player worker. Pure apart from the
// core and segmenter handed in; unit-tested in Node.
//
// LMS checks a sync group every couple of seconds and tells a player that is behind to skip
// ahead (`strm a`) and one that is ahead to pause briefly (`strm p` with an interval). In
// Web Audio mode the worklet applies them to the frame. The media element can't be
// corrected like that: seeking it (or pausing it) makes Safari drop the audio it has decoded
// and stall while it refills, which loses about as much time as the correction gained. The
// play point LMS sees then never improves, and it repeats the correction every check: the
// player drops out every two seconds for as long as the group plays.
//
// So the corrections are made in the audio itself, before it is muxed: a skip drops that
// much decoded PCM, and a pause inserts that much silence. The element plays on without a
// break. Both are heard only once the element reaches that point, after the audio already
// appended (see MEDIA_AHEAD_SECS in the worker), but both are reported to the core at once,
// as the worklet's are, so LMS's next check sees the correction made and does not ask again:
// - a skip counts the dropped frames as presented now. They never reach the element's
//   timeline, so the page's played reports never count them a second time;
// - a pause holds back that many frames of the page's next played reports (the core stops
//   extrapolating for the same interval). The silence is on the timeline and is counted as
//   it plays, which pays the held-back frames back: in all, only track audio is counted.
// Once the element has played past the correction, what the core reports is exactly what is
// heard again.

/** The core caps a correction at 10 s; anything more is not from LMS. */
export const MAX_CORRECTION_MS = 10_000;
/** Frames pulled or pushed at a time. */
const CHUNK = 4096;

const framesFor = (ms, rate) => Math.round((ms * rate) / 1000);
const validMs = (ms) => Number.isFinite(ms) && ms > 0 && ms <= MAX_CORRECTION_MS;
const validRate = (rate) => Number.isInteger(rate) && rate > 0 && rate <= 1_000_000;

export class MediaCorrections {
  /** Frames still to drop from the core's audio (more than it had decoded, so far). */
  #skip = 0;
  /** Frames of the page's played reports still to hold back (pauses reported already). */
  #debt = 0;

  /** A flush (or a new session): what was owed belongs to the old position. */
  reset() {
    this.#skip = 0;
    this.#debt = 0;
  }

  get pendingSkip() {
    return this.#skip;
  }

  get debt() {
    return this.#debt;
  }

  /**
   * LMS's skip of `ms` at `rate` Hz: drop that much of the core's decoded audio, counting it
   * as presented at once. Returns the frames dropped now (the rest goes as it is decoded:
   * see {@link dropDue}).
   */
  skip(ms, rate, player, at) {
    if (!validMs(ms) || !validRate(rate)) return 0;
    this.#skip = Math.min(this.#skip + framesFor(ms, rate), framesFor(MAX_CORRECTION_MS, rate));
    return this.dropDue(player, at);
  }

  /** Drop what is still owed of a skip, as far as the core has audio. Returns the frames dropped. */
  dropDue(player, at) {
    let dropped = 0;
    while (this.#skip > 0) {
      const want = Math.min(this.#skip, CHUNK);
      const n = player.pull_one(new Float32Array(want * 2));
      if (!n) break;
      this.#skip -= Math.min(n, this.#skip);
      dropped += n;
    }
    if (dropped > 0) player.presented(dropped, at);
    return dropped;
  }

  /**
   * LMS's pause of `ms` at `rate` Hz: push that much silence to the segmenter, as part of
   * `track`, and hold back as many frames of the played reports. Returns the frames of
   * silence pushed.
   */
  pause(ms, rate, segmenter, track) {
    if (!validMs(ms) || !validRate(rate) || !segmenter || !Number.isInteger(track) || track < 0) return 0;
    const total = framesFor(ms, rate);
    let frames = total;
    while (frames > 0) {
      const n = Math.min(frames, CHUNK);
      segmenter.push_track(new Float32Array(n * 2), track);
      frames -= n;
    }
    this.#debt += total;
    return total;
  }

  /** The page says the element played `frames`: how many of them the core is told of. */
  report(frames) {
    if (!Number.isSafeInteger(frames) || frames <= 0) return 0;
    const held = Math.min(frames, this.#debt);
    this.#debt -= held;
    return frames - held;
  }
}
