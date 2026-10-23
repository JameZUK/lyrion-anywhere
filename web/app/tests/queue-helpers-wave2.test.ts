// Queue helpers for Wave 2: swipes both ways, where Play next moves a track, Clear upcoming
// (from the end, each removal checked), and the shuffle cycle.
import { CLEAR_UPCOMING_MAX, ClearStopped, clearAfter, playNextTarget, removeAt, swipeAction, SWIPE_MIN_PX } from '#lib/lms/queue-edit';
import { nextShuffle, shuffleLabel, shuffleMode } from '#lib/lms/shuffle';
import * as c from '#lib/lms/commands';
import type { Track } from '#lib/lms/types';

test('a swipe acts only once it has gone far enough, either way', () => {
	expect(swipeAction(-SWIPE_MIN_PX, 100)).toBe('left');
	expect(swipeAction(SWIPE_MIN_PX, 100)).toBe('right');
	expect(swipeAction(SWIPE_MIN_PX - 1, 100)).toBeNull();
	// A wide row needs a longer swipe.
	expect(swipeAction(100, 1000)).toBeNull();
	expect(swipeAction(400, 1000)).toBe('right');
	for (const bad of [NaN, Infinity, -Infinity]) expect(swipeAction(bad, 100)).toBeNull();
	expect(swipeAction(90, NaN)).toBe('right');
});

test('Play next moves a track to just after the current one', () => {
	expect(playNextTarget(7, 2)).toBe(3);
	// From before the current track: the current one shifts down as it leaves.
	expect(playNextTarget(0, 2)).toBe(2);
});

function lms(tracks: Track[]) {
	let q = [...tracks];
	const sent: (string | number)[][] = [];
	const rpc = async (_p: string, cmd: (string | number)[]) => {
		sent.push(cmd);
		if (cmd[0] === 'status') {
			const from = Number(cmd[1]);
			return { playlist_loop: q.slice(from, from + 1).map((t) => ({ ...t, 'playlist index': from })) };
		}
		if (cmd[0] === 'playlist' && cmd[1] === 'delete') q = removeAt(q, Number(cmd[2]));
		return {};
	};
	return { rpc, sent, get q() {
		return q;
	}, set q(v: Track[]) {
		q = v;
	} };
}

const t = (n: number): Track => ({ id: n, title: `Song ${n}` });

test('Clear upcoming deletes from the end, each position checked first', async () => {
	const tracks = [t(1), t(2), t(3), t(4), t(5)];
	const fake = lms(tracks);
	const seen: number[] = [];
	const n = await clearAfter(fake.rpc, 'p', tracks, 1, (k) => seen.push(k));
	expect(n).toBe(3);
	expect(seen).toEqual([1, 2, 3]);
	expect(fake.q.map((x) => x.id)).toEqual([1, 2]);
	const deletes = fake.sent.filter((x) => x[0] === 'playlist').map((x) => x[2]);
	expect(deletes).toEqual([4, 3, 2]);
	// Every delete is preceded by a check of that position.
	fake.sent.forEach((x, i) => {
		if (x[0] === 'playlist') expect(fake.sent[i - 1]).toEqual(c.queue(Number(x[2]), 1));
	});
	// Nothing after the current track: nothing sent.
	const none = lms(tracks);
	expect(await clearAfter(none.rpc, 'p', tracks, 4)).toBe(0);
	expect(none.sent).toEqual([]);
});

test('Clear upcoming stops (and says how far it got) when the queue changed meanwhile', async () => {
	const tracks = [t(1), t(2), t(3), t(4), t(5)];
	const fake = lms(tracks);
	// Another app swapped track 3 for something else.
	fake.q = [t(1), t(2), t(99), t(4), t(5)];
	await expect(clearAfter(fake.rpc, 'p', tracks, 0)).rejects.toEqual(new ClearStopped(2));
	expect(fake.q.map((x) => x.id)).toEqual([1, 2, 99]);
	expect(new ClearStopped(1).message).toContain('1 track');
	expect(CLEAR_UPCOMING_MAX).toBeGreaterThan(10);
});

test('Clear upcoming passes other failures on', async () => {
	const rpc = async () => {
		throw new Error('offline');
	};
	await expect(clearAfter(rpc, 'p', [t(1), t(2)], 0)).rejects.toThrow('offline');
});

test('shuffle cycles Off → Tracks → Albums and says which', () => {
	expect(nextShuffle(0)).toBe(1);
	expect(nextShuffle(1)).toBe(2);
	expect(nextShuffle(2)).toBe(0);
	expect(shuffleLabel(0)).toBe('Shuffle: off');
	expect(shuffleLabel(1)).toBe('Shuffle: tracks');
	expect(shuffleLabel(2)).toBe('Shuffle: albums');
	for (const [v, m] of [[0, 0], [1, 1], [2, 2], ['2', 2], [' 1', 1], [3, 0], [-1, 0], ['x', 0], [null, 0], [undefined, 0], [1.5, 0]] as const) {
		expect(shuffleMode(v)).toBe(m);
	}
	expect(c.shuffle(2)).toEqual(['playlist', 'shuffle', 2]);
});
