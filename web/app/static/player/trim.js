// Partial flushes (LMS's `strm f`) at the player worker's outputs.
//
// LMS sends `strm f` when the next track, already streamed, is deleted or moved in its
// queue, and keeps the current track playing. The core keeps the audible track and drops
// what came after it; the worker must drop what it has already pulled of those tracks from
// its output too, or the deleted track plays anyway:
// - Web Audio: the last `drop_frames` frames sent to the worklet (still queued there);
// - the media element: the segments from where the kept track ends (the segmenter knows
//   where each track's audio went), and whatever the segmenter still holds of later tracks.

/**
 * What the worker does for the core's trim `t` (from `take_trim()`: `keep_track`,
 * `drop_frames`). `out` is the worker's output state: `mode` ('worklet' or 'media'),
 * `pulled` (frames pulled since the last flush), `segmenter` (media mode) and
 * `mediaWaiting` (a full flush is under way, which drops everything anyway).
 *
 * Returns `{ drop, pulled, worklet, media }`: the frames dropped, the new pulled count, the
 * message for the worklet (or null) and the messages for the page's media output, in order.
 * The segmenter is trimmed here.
 */
export function planTrim(t, out) {
  const raw = t && typeof t === 'object' ? t.drop_frames : 0;
  const pulled = Number.isSafeInteger(out.pulled) && out.pulled > 0 ? out.pulled : 0;
  const drop = Number.isSafeInteger(raw) && raw > 0 ? Math.min(raw, pulled) : 0;
  const plan = { drop, pulled: pulled - drop, worklet: null, media: [] };
  const keep = t && typeof t === 'object' ? t.keep_track : undefined;
  if (out.mode === 'media') {
    const seg = out.segmenter;
    if (!seg || out.mediaWaiting || !Number.isInteger(keep) || keep < 0) return plan;
    const rate = seg.rate();
    const from = seg.trim_after(keep);
    if (typeof from === 'number' && Number.isFinite(from)) plan.media.push({ trimFrom: from });
    if (seg.rate() !== rate) {
      // Back to the kept track's rate: the element needs its init segment again.
      plan.media.push({ init: seg.init_segment(), span: { at: seg.emitted_seconds(), rate: seg.rate() } });
    }
  } else if (drop > 0) {
    plan.worklet = { trim: drop };
  }
  return plan;
}
