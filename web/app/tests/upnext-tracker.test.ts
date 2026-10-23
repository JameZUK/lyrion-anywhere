// The per-device Up next tracker: Play next insertions, shifts from this device's own edits
// and from playing on, and dropping the sections on any change it can't account for.
import { PENDING_MS, UpNextTracker, gaplessLinks, stateOf } from '#lib/queue/tracker';

const P = 'aa:bb:cc:dd:ee:01';
const Q = 'aa:bb:cc:dd:ee:02';

function tracker(count = 10, cur = 2) {
	const t = new UpNextTracker();
	t.observe(P, { count, cur, shuffle: 0 }, 0);
	return t;
}

/** Play next of `n` tracks, landing on the next poll. */
function playNext(t: UpNextTracker, before: { count: number; cur: number }, n: number, at = 1) {
	t.expectInsert(P, { ...before, shuffle: 0 }, at);
	t.observe(P, { count: before.count + n, cur: before.cur, shuffle: 0 }, at + 1);
}

test('Play next grows Up next by however many tracks went in, right after the current one', () => {
	const t = tracker();
	expect(t.sections(P, 2, 10)).toBeNull();
	playNext(t, { count: 10, cur: 2 }, 3);
	expect(t.run(P)).toBe(3);
	expect(t.sections(P, 2, 13)).toEqual({ cur: 2, from: 3, to: 5 });
	playNext(t, { count: 13, cur: 2 }, 1, 10);
	expect(t.sections(P, 2, 14)).toEqual({ cur: 2, from: 3, to: 6 });
	// Another player's queue is separate.
	expect(t.run(Q)).toBe(0);
});

test('a Play next that lands a poll or two late still counts; one that never lands expires', () => {
	const t = tracker();
	t.expectInsert(P, { count: 10, cur: 2, shuffle: 0 }, 1);
	t.observe(P, { count: 10, cur: 2, shuffle: 0 }, 2);
	t.observe(P, { count: 12, cur: 2, shuffle: 0 }, 3);
	expect(t.run(P)).toBe(2);
	const u = tracker();
	u.expectInsert(P, { count: 10, cur: 2, shuffle: 0 }, 1);
	u.observe(P, { count: 12, cur: 2, shuffle: 0 }, 1 + PENDING_MS + 1);
	expect(u.run(P)).toBe(0);
});

test('playing on eats into Up next; going back drops it', () => {
	const t = tracker();
	playNext(t, { count: 10, cur: 2 }, 3);
	t.observe(P, { count: 13, cur: 3, shuffle: 0 }, 5);
	expect(t.sections(P, 3, 13)).toEqual({ cur: 3, from: 4, to: 5 });
	t.observe(P, { count: 13, cur: 6, shuffle: 0 }, 6);
	expect(t.run(P)).toBe(0);
	const u = tracker();
	playNext(u, { count: 10, cur: 2 }, 2);
	u.observe(P, { count: 12, cur: 1, shuffle: 0 }, 5);
	expect(u.run(P)).toBe(0);
});

test('changes it can’t account for drop the sections (flat queue)', () => {
	// Another app removed a track.
	const a = tracker();
	playNext(a, { count: 10, cur: 2 }, 2);
	a.observe(P, { count: 11, cur: 2, shuffle: 0 }, 5);
	expect(a.run(P)).toBe(0);
	// Reshuffled.
	const b = tracker();
	playNext(b, { count: 10, cur: 2 }, 2);
	b.observe(P, { count: 12, cur: 2, shuffle: 1 }, 5);
	expect(b.run(P)).toBe(0);
	// Grew without a Play next, before the run's tracks were ever checked.
	const c = tracker();
	playNext(c, { count: 10, cur: 2 }, 2);
	c.observe(P, { count: 15, cur: 2, shuffle: 0 }, 5);
	expect(c.run(P)).toBe(0);
	// Odd status: ignored.
	const d = tracker();
	playNext(d, { count: 10, cur: 2 }, 2);
	d.observe(P, { count: -1, cur: 2 }, 5);
	d.observe(P, { count: 3, cur: 7 }, 5);
	d.observe(P, null, 5);
	expect(d.run(P)).toBe(2);
});

test('grown at the end (a station topping up): kept while the run’s tracks still check out', () => {
	const t = tracker();
	playNext(t, { count: 10, cur: 2 }, 2);
	const ids = [1, 2, 3, 91, 92, 4, 5, 6, 7, 8, 9, 10];
	t.verify(P, 2, ids);
	t.observe(P, { count: 15, cur: 2, shuffle: 0 }, 5);
	t.verify(P, 2, [...ids, 11, 12, 13]);
	expect(t.run(P)).toBe(2);
	// …and dropped when they don't.
	t.observe(P, { count: 16, cur: 2, shuffle: 0 }, 6);
	t.verify(P, 2, [1, 2, 3, 4, 91, 92, 5]);
	expect(t.run(P)).toBe(0);
});

test('the queue page’s ids are checked: a reordered queue drops the sections', () => {
	const t = tracker();
	playNext(t, { count: 10, cur: 2 }, 2);
	t.verify(P, 2, [1, 2, 3, 91, 92, 4]);
	t.verify(P, 2, [1, 2, 3, 91, 92, 4]);
	expect(t.run(P)).toBe(2);
	t.verify(P, 2, [1, 2, 3, 92, 91, 4]);
	expect(t.run(P)).toBe(0);
});

test('this device’s removals shift the run', () => {
	const t = tracker();
	playNext(t, { count: 10, cur: 2 }, 3);
	// One of the run.
	t.removed(P, 4);
	expect(t.sections(P, 2, 12)).toEqual({ cur: 2, from: 3, to: 4 });
	// One after the run: no change, and the next poll (one fewer) is accounted for.
	t.removed(P, 9);
	t.observe(P, { count: 11, cur: 2, shuffle: 0 }, 5);
	expect(t.run(P)).toBe(2);
	// One before the current track: the current index moves down with it.
	t.removed(P, 0);
	t.observe(P, { count: 10, cur: 1, shuffle: 0 }, 6);
	expect(t.sections(P, 1, 10)).toEqual({ cur: 1, from: 2, to: 3 });
	// Out-of-range removals are ignored.
	t.removed(P, 99);
	t.removed(P, -1);
	expect(t.run(P)).toBe(2);
});

test('this device’s moves: into the slot after the current track joins the run; out of it leaves', () => {
	const t = tracker();
	playNext(t, { count: 10, cur: 2 }, 2); // run: 3, 4
	// Swipe right on track 8: moved to just after the current one.
	t.moved(P, 8, 3);
	expect(t.sections(P, 2, 12)).toEqual({ cur: 2, from: 3, to: 5 });
	// Move one of the run to the end: it leaves the run.
	t.moved(P, 3, 11);
	expect(t.run(P)).toBe(2);
	// A played track moved to play next (from before the current one).
	t.moved(P, 0, 2);
	expect(t.sections(P, 1, 12)).toEqual({ cur: 1, from: 2, to: 4 });
	// The next poll agrees: nothing dropped.
	t.observe(P, { count: 12, cur: 1, shuffle: 0 }, 9);
	expect(t.run(P)).toBe(3);
});

test('Play next on the track already next starts a run of one', () => {
	const t = tracker();
	t.moved(P, 3, 3);
	expect(t.run(P)).toBe(1);
});

test('clearing ends the run; reset forgets players', () => {
	const t = tracker();
	playNext(t, { count: 10, cur: 2 }, 2);
	t.truncated(P, 3);
	expect(t.run(P)).toBe(0);
	t.observe(P, { count: 3, cur: 2, shuffle: 0 }, 5);
	playNext(t, { count: 3, cur: 2 }, 1, 6);
	expect(t.run(P)).toBe(1);
	t.reset(P);
	expect(t.run(P)).toBe(0);
	t.reset();
});

test('undo (added at the end, then moved into place) is accounted for', () => {
	const t = tracker();
	playNext(t, { count: 10, cur: 2 }, 2);
	t.removed(P, 4); // count 11, run 1
	t.appended(P);
	t.moved(P, 11, 4);
	expect(t.run(P)).toBe(2);
	t.observe(P, { count: 12, cur: 2, shuffle: 0 }, 5);
	expect(t.run(P)).toBe(2);
});

test('an empty queue is never tracked; Play next onto it stays flat', () => {
	const t = new UpNextTracker();
	t.observe(P, { count: 0, cur: 0 }, 0);
	t.expectInsert(P, { count: 0, cur: 0 }, 1);
	t.observe(P, { count: 5, cur: 0 }, 2);
	expect(t.run(P)).toBe(0);
});

test('status reads as a queue state, odd values refused', () => {
	expect(stateOf({ playlist_tracks: 5, playlist_cur_index: '2', 'playlist shuffle': 1 })).toEqual({ count: 5, cur: 2, shuffle: 1 });
	expect(stateOf({ playlist_tracks: 5, playlist_cur_index: 9 })).toBeNull();
	expect(stateOf({ playlist_tracks: 'x' })).toBeNull();
	expect(stateOf(null)).toBeNull();
	expect(stateOf({})).toEqual({ count: 0, cur: 0 });
});

test('fuzz: any sequence of events never throws and keeps the run sane', () => {
	const t = new UpNextTracker();
	let count = 10;
	let cur = 0;
	for (let i = 0; i < 2000; i++) {
		const r = Math.random();
		if (r < 0.2) t.expectInsert(P, { count, cur }, i);
		else if (r < 0.4) {
			count += Math.floor(Math.random() * 3);
			t.observe(P, { count, cur }, i);
		} else if (r < 0.5) {
			if (count > 1) t.removed(P, Math.floor(Math.random() * count)), count--;
			if (cur >= count) cur = Math.max(0, count - 1);
		} else if (r < 0.6) t.moved(P, Math.floor(Math.random() * count), Math.floor(Math.random() * count));
		else if (r < 0.7) {
			cur = Math.min(count - 1, cur + 1);
			t.observe(P, { count, cur }, i);
		} else if (r < 0.75) t.verify(P, cur, Array.from({ length: count }, () => Math.floor(Math.random() * 4)));
		else t.observe(P, { count: Math.floor(Math.random() * 20), cur: Math.floor(Math.random() * 20), shuffle: Math.floor(Math.random() * 3) }, i);
		const k = t.run(P);
		expect(Number.isInteger(k) && k >= 0).toBe(true);
		const s = t.sections(P, cur, count);
		if (s) expect(s.from === s.cur + 1 && s.to >= s.from && s.to < count).toBe(true);
	}
});

test('gapless links: same album, next track number', () => {
	const rows = [
		{ album_id: 7, tracknum: 1 },
		{ album_id: 7, tracknum: '2' },
		{ album_id: 7, tracknum: 4 },
		{ album_id: 8, tracknum: 5 },
		{ album_id: 8, tracknum: 6 },
		{ tracknum: 7 },
		{ tracknum: 8 },
	];
	expect(gaplessLinks(rows)).toEqual([true, false, false, true, false, false, false]);
	expect(gaplessLinks([])).toEqual([]);
	for (const odd of [[{ album_id: 'x', tracknum: 1 }, { album_id: 'x', tracknum: 2 }], [{ album_id: 1, tracknum: '1-2' }, { album_id: 1, tracknum: '2' }], [null as never, { album_id: 1 }]]) {
		expect(() => gaplessLinks(odd)).not.toThrow();
		expect(gaplessLinks(odd).every((x) => x === false)).toBe(true);
	}
});
