// Recent searches: on this device only, at most 20, newest first, no repeats, each removable,
// kept per user, and robust to whatever storage holds.
import { MAX_RECENTS, MAX_TERM, addRecent, cleanTerm, loadRecents, parseRecents, removeRecent, saveRecents } from '#lib/search/recents';

function memory() {
	const m = new Map<string, string>();
	return {
		getItem: (k: string) => m.get(k) ?? null,
		setItem: (k: string, v: string) => void m.set(k, v),
		removeItem: (k: string) => void m.delete(k),
		m,
	};
}

test('newest first, a repeat (in any case) moves to the front, capped at 20', () => {
	let l: string[] = [];
	l = addRecent(l, 'alpha');
	l = addRecent(l, 'beta');
	l = addRecent(l, 'ALPHA');
	expect(l).toEqual(['ALPHA', 'beta']);
	for (let i = 0; i < 30; i++) l = addRecent(l, `term ${i}`);
	expect(l).toHaveLength(MAX_RECENTS);
	expect(l[0]).toBe('term 29');
});

test('terms are cleaned: control characters, spacing and length', () => {
	expect(cleanTerm('  a\u0000b\n c  ')).toBe('a b c');
	expect(cleanTerm('x'.repeat(500))).toHaveLength(MAX_TERM);
	expect(cleanTerm(42)).toBe('');
	expect(addRecent(['a'], '   ')).toEqual(['a']);
	expect(addRecent(['a'], null)).toEqual(['a']);
});

test('one removed; the rest kept', () => {
	expect(removeRecent(['a', 'B', 'c'], 'b')).toEqual(['a', 'c']);
	expect(removeRecent([], 'b')).toEqual([]);
});

test('kept per user and per device; never anything but a clean list', () => {
	const s = memory();
	saveRecents('ann', ['one', 'two'], s);
	saveRecents('bob', ['three'], s);
	expect(loadRecents('ann', s)).toEqual(['one', 'two']);
	expect(loadRecents('bob', s)).toEqual(['three']);
	expect(loadRecents('', s)).toEqual([]);
	// Clear all removes the key.
	saveRecents('ann', [], s);
	expect([...s.m.keys()].some((k) => k.endsWith('.ann'))).toBe(false);
	for (const junk of ['{', '7', '{"a":1}', '[1,null,{},"ok","OK"]', JSON.stringify(Array.from({ length: 300 }, (_, i) => `t${i}`))]) {
		s.m.set([...s.m.keys()][0]!, junk);
		const out = loadRecents('bob', s);
		expect(Array.isArray(out)).toBe(true);
		expect(out.length).toBeLessThanOrEqual(MAX_RECENTS);
		for (const t of out) expect(typeof t).toBe('string');
	}
	expect(parseRecents([1, null, {}, 'ok', 'OK'])).toEqual(['ok']);
});

test('a user name can’t reach another key', () => {
	const s = memory();
	saveRecents('../x', ['a'], s);
	expect([...s.m.keys()][0]).toBe('anywhere.search.recents...%2Fx');
});

test('blocked storage never throws', () => {
	const blocked = {
		getItem: () => {
			throw new Error('blocked');
		},
		setItem: () => {
			throw new Error('blocked');
		},
		removeItem: () => {
			throw new Error('blocked');
		},
	};
	expect(loadRecents('ann', blocked)).toEqual([]);
	expect(() => saveRecents('ann', ['a'], blocked)).not.toThrow();
	expect(() => saveRecents('ann', [], blocked)).not.toThrow();
});
