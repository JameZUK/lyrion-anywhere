// Connection-log lines for what the core and LMS say to each other, and for the output's
// back-pressure (issue #12: seeking on iPadOS through a tunnel). Pure functions, unit-tested
// in Node. The page rate-limits each line by its kind (see diagnostics.ts, LogLimiter).

/** One printable character from the core, else '?'. */
const letter = (c) => (typeof c === 'string' && /^[!-~]$/.test(c) ? c : '?');

/**
 * A `strm` from LMS (from WasmPlayer.take_diagnostics): the command letter, and for `s` the
 * autostart flag, format letter and threshold (KiB). Nothing else from the command.
 */
export function describeStrm(e) {
  const cmd = letter(e?.command);
  if (cmd !== 's') return `lms: strm ${cmd}`;
  const kib = Number.isInteger(e.threshold) && e.threshold >= 0 && e.threshold <= 255 ? e.threshold : '?';
  return `lms: strm s autostart=${letter(e.autostart)} format=${letter(e.format)} threshold=${kib}`;
}

/** What each STAT the core sends tells LMS. */
export const STAT_GLOSS = {
  STMc: 'connecting to the stream',
  STMs: 'the track started playing',
  STMl: 'loaded, waiting for LMS to start it',
  STMp: 'paused, as LMS asked',
  STMr: 'resumed playing',
  STMo: 'the output ran dry',
  STMu: 'played out, nothing queued',
  STMd: 'decoded, ready for the next track',
  STMf: 'flushed, as LMS asked',
  STMn: 'format not supported',
};

/** A STAT the core sent, e.g. `player: STMl (loaded, waiting for LMS to start it)`; null if unknown. */
export function describeStat(event) {
  if (typeof event !== 'string' || !/^STM[a-z]$/.test(event)) return null;
  const gloss = STAT_GLOSS[event];
  return gloss ? `player: ${event} (${gloss})` : `player: ${event}`;
}

/** The log line for one of the core's diagnostics events, with its rate-limit kind; null to skip it. */
export function describeDiag(e) {
  if (e?.type === 'strm') return { line: describeStrm(e), kind: `strm:${letter(e.command)}` };
  if (e?.type === 'stat') {
    const line = describeStat(e.event);
    return line ? { line, kind: `stat:${e.event}` } : null;
  }
  if (e?.type === 'flac_header') {
    // LMS serves a FLAC seek as bare frames; the core made the missing header (issue #12).
    const n = (v) => (Number.isFinite(v) && v > 0 ? Math.round(v) : '?');
    return { line: `player: the stream had no FLAC header (a seek is sent so); made one from its first frame: ${n(e.rate)} Hz, ${n(e.bits)}-bit, ${n(e.channels)} ch`, kind: undefined };
  }
  return null;
}

/** The core's output state by name: 'Paused { until: None }' is 'Paused'. */
export function outputName(output) {
  const s = String(output ?? '');
  const m = s.match(/^[A-Z][a-z]+/);
  return m ? m[0] : '?';
}

/** A pause that ends by itself (a sync pause, or a timed start). */
const timed = (output) => /^Paused \{ until: Some/.test(String(output ?? ''));

// A `strm` this recent (ms) is taken as the cause of an output change.
const STRM_CAUSE_MS = 2000;

/**
 * Why the output went from `from` to `to` (names from {@link outputName}), if known. `raw` is
 * the core's own state now (before a hold made on this device), `strm` the last `strm`
 * received `{ command, at }`, and `now` the time (ms, the same clock).
 */
export function outputCause(from, to, raw, strm, now) {
  if (to === 'Held') return 'local pause';
  if (from === 'Held') return 'local resume';
  const recent = strm && now - strm.at <= STRM_CAUSE_MS ? letter(strm.command) : null;
  if (to === 'Paused' && timed(raw)) return recent === 'u' ? 'timed start, after strm u' : 'sync';
  if (recent) return `after strm ${recent}`;
  if (from === 'Paused' && to === 'Playing') return 'timed pause ended';
  if (from === 'Waiting' && (to === 'Playing' || to === 'Loaded')) return 'buffer ready';
  return null;
}

/** `player: Playing → Paused (after strm p)`. */
export function describeOutputChange(from, to, cause) {
  return `player: ${from} → ${to}${cause ? ` (${cause})` : ''}`;
}

/** Seconds to one decimal place. */
export const secs = (s) => (Number.isFinite(s) ? Math.max(0, s) : 0).toFixed(1);

/**
 * A condition that matters only once it has lasted `limit` ms (an element that stopped
 * playing, a download parked). `update(active, now)` says 'began' once when it has lasted
 * that long, and 'ended' once when it clears after that (`lasted` then says for how long, in
 * ms); otherwise null.
 */
export class Persisting {
  constructor(limit) {
    this.limit = limit;
    this.since = -1;
    this.reported = false;
    this.lasted = 0;
  }

  update(active, now) {
    if (!active) {
      const was = this.reported;
      this.lasted = this.since >= 0 ? now - this.since : 0;
      this.since = -1;
      this.reported = false;
      return was ? 'ended' : null;
    }
    if (this.since < 0) this.since = now;
    if (!this.reported && now - this.since >= this.limit) {
      this.reported = true;
      return 'began';
    }
    return null;
  }

  reset() {
    this.since = -1;
    this.reported = false;
  }
}

/** `media: element not consuming (lead 8.0 s, buffered 14.3 s)`. */
export function describeNotConsuming(leadSecs, bufferedSecs) {
  return `media: element not consuming (lead ${secs(leadSecs)} s, buffered ${secs(bufferedSecs)} s)`;
}

/**
 * What the worker did with the page's "media-drained" (the element played all it was given):
 * `how` is 'played-out', 'flushed' (with `frames` sent) or 'ignored' (with `why`).
 */
export function describeDrained(how, detail) {
  if (how === 'played-out') return 'media: drained, counted as played out';
  if (how === 'flushed') return `media: drained, flushed ${Number.isInteger(detail) ? detail : 0} pending frames`;
  return `media: drained, ignored (${detail})`;
}
