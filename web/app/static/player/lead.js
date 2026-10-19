// How far ahead of the play head the media element is fed, and how much played audio is
// kept behind it, in seconds: from the stream's data rate, so hi-res audio does not sit on
// the browser's media buffer quota (issue #12 follow-up).
//
// Safari gives an audio-only SourceBuffer about 5 % of its media budget: some 5.5 MB on an
// iPad. The element is fed FLAC verbatim frames, 576 KB/s at 24/96 and 1.15 MB/s at 24/192,
// so eight seconds ahead plus ten behind is far over that, and every append then waits for
// the play head. Fixed byte budgets, clamped to sensible seconds, keep the whole buffer
// under the quota at any rate while CD quality keeps its full lead.

/** Lead at low data rates (seconds): the most ever fed ahead. */
export const MEDIA_AHEAD_SECS = 8;
/** The least lead at any rate (seconds): enough to ride out a tick or two of decoding. */
export const MIN_AHEAD_SECS = 3;
/** Bytes fed ahead of the play head at most (above MIN_AHEAD_SECS). */
export const MEDIA_AHEAD_BYTES = 2.5 * 1024 * 1024;
/** Played audio kept behind the play head at low data rates (seconds). */
export const KEEP_BEHIND_SECS = 10;
/** The least kept behind at any rate (seconds). */
export const MIN_BEHIND_SECS = 1;
/** Bytes kept behind the play head at most (above MIN_BEHIND_SECS). */
export const KEEP_BEHIND_BYTES = 1024 * 1024;

function clampSecs(bytes, bytesPerSec, min, max) {
  if (!(bytesPerSec > 0) || !Number.isFinite(bytesPerSec)) return max;
  return Math.min(max, Math.max(min, bytes / bytesPerSec));
}

/** Seconds to feed ahead of the play head for a stream of `bytesPerSec`. */
export function mediaAheadSecs(bytesPerSec) {
  return clampSecs(MEDIA_AHEAD_BYTES, bytesPerSec, MIN_AHEAD_SECS, MEDIA_AHEAD_SECS);
}

/** Seconds of played audio to keep behind the play head for a stream of `bytesPerSec`. */
export function keepBehindSecs(bytesPerSec) {
  return clampSecs(KEEP_BEHIND_BYTES, bytesPerSec, MIN_BEHIND_SECS, KEEP_BEHIND_SECS);
}
