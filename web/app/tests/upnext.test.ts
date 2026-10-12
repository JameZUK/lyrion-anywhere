// "Up next" (docs/plans: Wave 1 §3): the track after the current one, from `status - 2`.
import { upNext } from '$lib/lms/upnext';
import type { PlayerStatus } from '$lib/lms/types';

const cur = { id: 1, title: 'Now', 'playlist index': 3 };
const next = { id: 2, title: 'Next', 'playlist index': 4 };

test('the second entry of the status, at the current index + 1', () => {
	const s: PlayerStatus = { playlist_cur_index: '3', playlist_loop: [cur, next] };
	expect(upNext(s)).toEqual({ track: next, index: 4 });
	// LMS left out the index: trusted to be the next one.
	expect(upNext({ playlist_cur_index: 3, playlist_loop: [cur, { id: 2, title: 'Next' }] })?.index).toBe(4);
});

test('nothing after the current track, or a status that does not line up: none', () => {
	expect(upNext(null)).toBeNull();
	expect(upNext({ playlist_cur_index: 3, playlist_loop: [cur] })).toBeNull();
	expect(upNext({ playlist_cur_index: 3, playlist_loop: [] })).toBeNull();
	expect(upNext({ playlist_cur_index: 3 })).toBeNull();
	// Out of step (an old answer): none rather than the wrong track.
	expect(upNext({ playlist_cur_index: 3, playlist_loop: [cur, { ...next, 'playlist index': 9 }] })).toBeNull();
	// No current index.
	expect(upNext({ playlist_loop: [cur, next] })).toBeNull();
	expect(upNext({ playlist_cur_index: 'x', playlist_loop: [cur, next] })).toBeNull();
	expect(upNext({ playlist_cur_index: -1, playlist_loop: [cur, next] })).toBeNull();
});

test('repeat this track: the same track plays again, so nothing is "up next"', () => {
	expect(upNext({ playlist_cur_index: 3, 'playlist repeat': 1, playlist_loop: [cur, next] })).toBeNull();
	expect(upNext({ playlist_cur_index: 3, 'playlist repeat': 2, playlist_loop: [cur, next] })?.track).toBe(next);
});

test('junk in the loop never throws', () => {
	for (const junk of [null, 7, 'x', [], {}]) {
		expect(() => upNext({ playlist_cur_index: 0, playlist_loop: [cur, junk as never] })).not.toThrow();
	}
	expect(upNext({ playlist_cur_index: 0, playlist_loop: 'abc' as never })).toBeNull();
});
