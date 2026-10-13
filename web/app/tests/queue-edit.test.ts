// Queue editing logic: the optimistic list must match what LMS does with `playlist move`, and
// the gesture maths must be sane for any input.
import { QueueChanged, SWIPE_MIN_PX, checkAt, dropIndex, insertAt, isSwipe, moveChecked, removeAt, removeChecked, reorder, sameTrack, swipeRemoves } from '$lib/lms/queue-edit';

/** What LMS's playlist move does: take the item out, put it in so it ends up at `to`. */
function lmsMove<T>(list: T[], from: number, to: number): T[] {
	const copy = [...list];
	const [x] = copy.splice(from, 1);
	copy.splice(to, 0, x as T);
	return copy;
}

test('reorder matches LMS playlist move, both ways', () => {
	const q = ['a', 'b', 'c', 'd', 'e'];
	expect(reorder(q, 0, 2)).toEqual(['b', 'c', 'a', 'd', 'e']);
	expect(reorder(q, 4, 1)).toEqual(['a', 'e', 'b', 'c', 'd']);
	expect(reorder(q, 2, 2)).toEqual(q);
	for (let f = 0; f < q.length; f++) for (let t = 0; t < q.length; t++) expect(reorder(q, f, t)).toEqual(lmsMove(q, f, t));
	expect(q, 'input untouched').toEqual(['a', 'b', 'c', 'd', 'e']);
});

test('out-of-range or odd indexes leave the list as it was', () => {
	const q = [1, 2, 3];
	for (const [f, t] of [[-1, 0], [0, 3], [3, 0], [0.5, 1], [NaN, 1], [1, Infinity]] as [number, number][]) {
		expect(reorder(q, f, t)).toEqual(q);
	}
	expect(removeAt(q, 5)).toEqual(q);
	expect(removeAt(q, -1)).toEqual(q);
	expect(removeAt(q, 1)).toEqual([1, 3]);
	expect(insertAt([1, 3], 1, 2)).toEqual([1, 2, 3]);
	expect(insertAt([1], 99, 2)).toEqual([1, 2]);
	expect(insertAt([1], -4, 0)).toEqual([0, 1]);
});

test('drop index counts the rows whose middle is above the dragged row', () => {
	const rows = [0, 1, 2, 3].map((i) => ({ top: i * 50, height: 50 }));
	expect(dropIndex(rows, 0, 25)).toBe(0); // not moved
	expect(dropIndex(rows, 0, 80)).toBe(1); // past row 1's middle (75)
	expect(dropIndex(rows, 0, 140)).toBe(2);
	expect(dropIndex(rows, 0, 9999)).toBe(3);
	expect(dropIndex(rows, 3, -50)).toBe(0);
	expect(dropIndex(rows, 3, 70)).toBe(1);
	expect(dropIndex(rows, 2, NaN)).toBe(2);
	expect(dropIndex([], 0, 10)).toBe(0);
});

test('swipe threshold: far enough left, scaled to the row', () => {
	expect(swipeRemoves(-SWIPE_MIN_PX, 0)).toBe(true);
	expect(swipeRemoves(-SWIPE_MIN_PX + 1, 0)).toBe(false);
	expect(swipeRemoves(-100, 400)).toBe(false); // needs 35% of 400 = 140
	expect(swipeRemoves(-140, 400)).toBe(true);
	expect(swipeRemoves(200, 400), 'rightwards never removes').toBe(false);
	expect(swipeRemoves(NaN, 400)).toBe(false);
	expect(swipeRemoves(-Infinity, 400)).toBe(false);
	expect(swipeRemoves(-90, NaN)).toBe(true);
});

test('a swipe is clearly sideways and past a dead zone', () => {
	expect(isSwipe(5, 0)).toBe(false);
	expect(isSwipe(-20, 2)).toBe(true);
	expect(isSwipe(-20, 15)).toBe(false); // diagonal: let the page scroll
	expect(isSwipe(0, -100)).toBe(false);
});

test('robust on arbitrary input: never throws, always a permutation', () => {
	let seed = 7;
	const rnd = () => ((seed = (seed * 1103515245 + 12345) % 2 ** 31) / 2 ** 31);
	const odd = [NaN, Infinity, -Infinity, -1, 0.5, 1e9, -0];
	for (let n = 0; n < 2000; n++) {
		const len = Math.floor(rnd() * 8);
		const q = Array.from({ length: len }, (_, i) => i);
		const pick = () => (rnd() < 0.2 ? odd[Math.floor(rnd() * odd.length)]! : Math.floor(rnd() * (len + 2)) - 1);
		const f = pick();
		const t = pick();
		const r = reorder(q, f, t);
		expect([...r].sort((a, b) => a - b)).toEqual(q);
		expect(removeAt(q, f).length).toBeGreaterThanOrEqual(Math.max(0, len - 1));
		const rows = q.map(() => ({ top: rnd() * 500, height: rnd() * 80 }));
		const d = dropIndex(rows, f, rnd() * 600 - 50);
		expect(d).toBeGreaterThanOrEqual(0);
		expect(d).toBeLessThanOrEqual(Math.max(0, len));
	}
});

describe('checked edits', () => {
	const at = (q: { id?: number; title?: string }[]) => async (_p: string, c: (string | number)[]) => {
		if (c[0] === 'status') {
			const from = Number(c[1]);
			return { playlist_loop: q.slice(from, from + Number(c[2])).map((t, i) => ({ ...t, 'playlist index': from + i })) };
		}
		return {};
	};

	test('same track: by id, or by title for entries without one', () => {
		expect(sameTrack({ id: 3, title: 'a' }, { id: 3, title: 'b' })).toBe(true);
		expect(sameTrack({ id: 3 }, { id: 4 })).toBe(false);
		expect(sameTrack({ id: 3 }, { title: 'x' })).toBe(false);
		expect(sameTrack({ title: 'Radio' }, { title: 'Radio' })).toBe(true);
		expect(sameTrack(undefined, { id: 1 })).toBe(false);
	});

	test('removes only when the track is still at that position', async () => {
		const sent: (string | number)[][] = [];
		const q = [{ id: 1 }, { id: 2 }, { id: 3 }];
		const rpc = async (p: string, c: (string | number)[]) => (sent.push(c), at(q)(p, c));
		await removeChecked(rpc, 'p', 1, { id: 2 });
		expect(sent.at(-1)).toEqual(['playlist', 'delete', 1]);
		sent.length = 0;
		await expect(removeChecked(rpc, 'p', 1, { id: 3 })).rejects.toBeInstanceOf(QueueChanged);
		await expect(removeChecked(rpc, 'p', 7, { id: 3 })).rejects.toBeInstanceOf(QueueChanged);
		await expect(moveChecked(rpc, 'p', 0, 2, { id: 2 })).rejects.toBeInstanceOf(QueueChanged);
		expect(sent.filter((c) => c[0] !== 'status')).toEqual([]);
		await moveChecked(rpc, 'p', 0, 2, { id: 1 });
		expect(sent.at(-1)).toEqual(['playlist', 'move', 0, 2]);
	});

	test('bad positions and odd answers are refused, never sent', async () => {
		const sent: (string | number)[][] = [];
		const rpc = async (_p: string, c: (string | number)[]) => (sent.push(c), null);
		for (const i of [-1, 1.5, NaN]) await expect(checkAt(rpc, 'p', i, { id: 1 })).rejects.toBeInstanceOf(QueueChanged);
		await expect(removeChecked(rpc, 'p', 0, { id: 1 })).rejects.toBeInstanceOf(QueueChanged);
		const wrongPos = async () => ({ playlist_loop: [{ id: 1, 'playlist index': 5 }] });
		await expect(checkAt(wrongPos, 'p', 0, { id: 1 })).rejects.toBeInstanceOf(QueueChanged);
		expect(sent.filter((c) => c[0] !== 'status')).toEqual([]);
	});
});
