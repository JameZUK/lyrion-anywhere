// The windowed pager behind album and artist lists: pages on, jumps to an offset without
// loading up to it, pages back, keeps the A–Z index, and copes with odd answers.
import { WindowPager } from '#lib/lms/pager.svelte';

type Row = { id: number | null } | null;
function server(total = 50) {
	const asked: [number, number, boolean][] = [];
	const fetch = async (from: number, n: number, first: boolean) => {
		asked.push([from, n, first]);
		const rows = Array.from({ length: Math.max(0, Math.min(n, total - from)) }, (_, i) => ({ id: from + i + 1 }));
		return { rows, count: total, ...(first ? { indexList: [['A', 20], ['B', 30]] } : {}) };
	};
	return { fetch, asked };
}

test('pages on, jumps, and pages back', async () => {
	const s = server();
	const p = new WindowPager<Row>(s.fetch, (x) => x?.id ?? null, 10);
	await p.reset();
	expect(p.index.map((e) => e.offset)).toEqual([0, 20]);
	expect(p.items).toHaveLength(10);
	await p.jumpTo(5); // already loaded
	expect(s.asked).toHaveLength(1);
	await p.jumpTo(35);
	expect(p.base).toBe(35);
	expect(p.items[0]).toEqual({ id: 36 });
	expect(p.index).toHaveLength(2); // kept across the jump
	await p.more();
	expect(p.items).toHaveLength(15);
	expect(p.done).toBe(true);
	await p.earlier();
	expect(p.base).toBe(25);
	expect(p.items[0]).toEqual({ id: 26 });
	await p.earlier();
	await p.earlier();
	await p.earlier();
	expect(p.base).toBe(0);
	expect(p.items).toHaveLength(50);
	expect(s.asked.every(([from, n]) => from >= 0 && n >= 1)).toBe(true);
});

test('bad offsets are ignored; odd answers give an empty list', async () => {
	const s = server();
	const p = new WindowPager<Row>(s.fetch, (x) => x?.id ?? null, 10);
	await p.reset();
	for (const bad of [-1, NaN, 1.5, Infinity]) await p.jumpTo(bad);
	expect(p.base).toBe(0);
	const q = new WindowPager<Row>(async () => ({ rows: 'x' as unknown as Row[], count: -3, indexList: 'junk' }), (x) => x?.id ?? null);
	await q.reset();
	expect(q.items).toEqual([]);
	expect(q.total).toBe(0);
	expect(q.index).toEqual([]);
	const r = new WindowPager<Row>(async () => {
		throw new Error('refused');
	}, (x) => x?.id ?? null);
	await r.reset();
	expect(r.error).toBe('refused');
});

test('a single batch never pages', async () => {
	const s = server(500);
	const p = new WindowPager<Row>(s.fetch, (x) => x?.id ?? null, 60, true);
	await p.reset();
	expect(p.total).toBe(60);
	expect(p.done).toBe(true);
	await p.more();
	expect(s.asked).toHaveLength(1);
});
