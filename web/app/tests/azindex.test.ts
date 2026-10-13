// The A–Z index: LMS's indexList as offsets (running totals), robust on any input.
import { MAX_KEYS, keyAt, keyFromPointer, parseIndexList } from '$lib/lms/azindex';
import * as c from '$lib/lms/commands';

test('each letter starts where the counts before it add up to', () => {
	const idx = parseIndexList([
		['A', 3],
		['B', 2],
		['D', 5],
	]);
	expect(idx).toEqual([
		{ key: 'A', count: 3, offset: 0 },
		{ key: 'B', count: 2, offset: 3 },
		{ key: 'D', count: 5, offset: 5 },
	]);
	expect(keyAt(idx, 0)?.key).toBe('A');
	expect(keyAt(idx, 4)?.key).toBe('B');
	expect(keyAt(idx, 9)?.key).toBe('D');
	expect(keyAt(idx, 10)).toBeNull();
	expect(keyAt(idx, -1)).toBeNull();
});

test('counts given as text are read; empty or bad entries are skipped but still ordered', () => {
	const idx = parseIndexList([['A', '2'], ['B', 0], 'junk', null, ['C'], ['C', 4], [{}, 1], ['', 2]]);
	expect(idx.map((e) => [e.key, e.offset, e.count])).toEqual([
		['A', 0, 2],
		['C', 2, 4],
		['#', 6, 3],
	]);
});

test('keys are one printable character; repeats are merged', () => {
	const idx = parseIndexList([
		['Ab', 1],
		['\u0000A', 1],
		['😀x', 2],
	]);
	expect(idx.map((e) => e.key)).toEqual(['A', '😀']);
	expect(idx[0]).toEqual({ key: 'A', count: 2, offset: 0 });
	expect(idx[1]?.offset).toBe(2);
});

test('odd or hostile input never throws and stays bounded', () => {
	for (const v of [null, undefined, 1, 'A', {}, [[]], [[1, 2, 3]], [['A', 1e300]], [['A', -5]], [['A', NaN]], [['A', '1e3']]]) {
		expect(() => parseIndexList(v)).not.toThrow();
	}
	const huge = Array.from({ length: 50_000 }, (_, i) => [String.fromCharCode(0x4e00 + i), 1]);
	const idx = parseIndexList(huge);
	expect(idx.length).toBeLessThanOrEqual(MAX_KEYS);
	// Everything is still accounted for (the last letter takes the rest).
	expect(idx.reduce((s, e) => s + e.count, 0)).toBe(10_000);
	for (let i = 0; i < 300; i++) {
		const v = Array.from({ length: 5 }, () => [Math.random() < 0.5 ? 'X' : Math.random(), Math.random() < 0.5 ? Math.floor(Math.random() * 9) : 'z']);
		const out = parseIndexList(v);
		for (const e of out) expect(Number.isSafeInteger(e.offset) && e.count > 0).toBe(true);
	}
});

test('a pointer over the strip picks the letter under it, clamped to the ends', () => {
	expect(keyFromPointer(26, 100, 260, 100)).toBe(0);
	expect(keyFromPointer(26, 100, 260, 115)).toBe(1);
	expect(keyFromPointer(26, 100, 260, 359)).toBe(25);
	expect(keyFromPointer(26, 100, 260, 5000)).toBe(25);
	expect(keyFromPointer(26, 100, 260, -50)).toBe(0);
	expect(keyFromPointer(0, 0, 100, 50)).toBe(0);
	expect(keyFromPointer(5, 0, 0, 50)).toBe(0);
	expect(keyFromPointer(5, 0, 100, NaN)).toBe(0);
});

test('the index is asked for with tags:Z, only for an A–Z album list and for artists', () => {
	expect(c.albums(0, 100, 'album', undefined, {}, true)).toEqual(['albums', 0, 100, `${c.ALBUM_TAGS}Z`, 'sort:album']);
	expect(c.albums(0, 100, 'new', undefined, {}, true)).toEqual(['albums', 0, 100, c.ALBUM_TAGS, 'sort:new']);
	expect(c.artists(0, 100, undefined, {}, true)).toEqual(['artists', 0, 100, 'tags:Z']);
	expect(c.artists(0, 100, undefined, { role: 'ALBUMARTIST' }, true)).toEqual(['artists', 0, 100, 'tags:Z', 'role_id:ALBUMARTIST']);
});
