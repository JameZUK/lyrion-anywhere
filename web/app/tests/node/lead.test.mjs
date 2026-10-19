// The media element's lead and keep-behind from the stream's data rate (issue #12 follow-up).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { KEEP_BEHIND_SECS, MEDIA_AHEAD_SECS, MIN_AHEAD_SECS, MIN_BEHIND_SECS, keepBehindSecs, mediaAheadSecs } from '../../static/player/lead.js';

const CD = 44100 * 2 * 2; // 176,400 B/s of 16-bit verbatim frames
const HIRES = 96000 * 2 * 3; // 576,000 B/s at 24/96
const ULTRA = 192000 * 2 * 3; // 1,152,000 B/s at 24/192

test('CD quality keeps the full lead and keep-behind', () => {
  assert.equal(mediaAheadSecs(CD), MEDIA_AHEAD_SECS);
  assert.equal(keepBehindSecs(CD), 5.944308390022676); // 1 MiB behind: under the 10 s cap
});

test('24/96 stays well under an iPad\'s 5.5 MB media quota', () => {
  const ahead = mediaAheadSecs(HIRES);
  const behind = keepBehindSecs(HIRES);
  assert.ok(ahead > 4 && ahead < 5, `${ahead} s ahead`);
  assert.ok(behind > 1.5 && behind < 2, `${behind} s behind`);
  assert.ok((ahead + behind) * HIRES < 4.5e6, 'ahead plus behind fits with room to spare');
});

test('24/192 lands on the floors, still under the quota', () => {
  assert.equal(mediaAheadSecs(ULTRA), MIN_AHEAD_SECS);
  assert.equal(keepBehindSecs(ULTRA), MIN_BEHIND_SECS);
  assert.ok((MIN_AHEAD_SECS + MIN_BEHIND_SECS) * ULTRA < 5e6);
});

test('an unknown or junk data rate means the low-rate defaults', () => {
  for (const x of [0, -1, NaN, Infinity, undefined, null]) {
    assert.equal(mediaAheadSecs(x), MEDIA_AHEAD_SECS);
    assert.equal(keepBehindSecs(x), KEEP_BEHIND_SECS);
  }
});
