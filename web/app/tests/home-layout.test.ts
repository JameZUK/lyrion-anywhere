// The Home layout: edits, and the same checks as the gateway (gateway/src/home.rs).
import * as L from '#lib/home/layout';
import type { HomeLayout } from '#lib/api/me';

const types = (l: HomeLayout) => l.shelves.map((s) => s.type);

test('the default is the agreed order', () => {
	expect(types(L.defaultLayout())).toEqual(['recent', 'stations', 'new', 'most', 'on_this_day']);
	expect(L.layoutProblem(L.defaultLayout())).toBeNull();
});

test('moving, hiding, removing and adding', () => {
	let l = L.defaultLayout();
	l = L.moveDown(l, 0);
	expect(types(l).slice(0, 2)).toEqual(['stations', 'recent']);
	l = L.moveUp(l, 1);
	expect(types(l)[0]).toBe('recent');
	expect(L.moveUp(l, 0)).toEqual(l);
	expect(types(L.moveDown(l, 4))).toEqual(types(l));
	expect(types(L.move(l, 4, 0))[0]).toBe('on_this_day');
	expect(L.move(l, 9, 0)).toBe(l);
	l = L.toggleHidden(l, 2);
	expect(l.shelves[2]).toEqual({ type: 'new', hidden: true });
	expect(L.toggleHidden(l, 2).shelves[2]!.hidden).toBe(false);
	l = L.remove(l, 0);
	expect(types(l)).not.toContain('recent');
	l = L.add(l, { type: 'station', id: 4 });
	l = L.add(l, { type: 'recent' });
	expect(types(l).slice(-2)).toEqual(['station', 'recent']);
	expect(() => L.add(l, { type: 'recent' })).toThrow('already on Home');
	expect(() => L.add(l, { type: 'station', id: 4 })).toThrow('already on Home');
	expect(types(L.add(l, { type: 'station', id: 5 })).filter((t) => t === 'station')).toHaveLength(2);
	expect(L.addable(l)).not.toContain('recent');
	expect(L.addable(l)).toContain('station');
});

test('the saved copy carries only what the gateway takes', () => {
	const l = { version: 1 as const, default: true, shelves: [{ type: 'new' as const, hidden: false }, { type: 'genre' as const, id: 3, hidden: true }] };
	expect(L.clean(l)).toEqual({ version: 1, shelves: [{ type: 'new' }, { type: 'genre', id: 3, hidden: true }] });
});

test('hostile layouts are refused', () => {
	const shelf = (s: unknown) => ({ version: 1, shelves: [s] });
	const bad: unknown[] = [
		null,
		[],
		'x',
		{},
		{ version: 2, shelves: [] },
		{ version: '1', shelves: [] },
		{ version: 1 },
		{ version: 1, shelves: {} },
		shelf('recent'),
		shelf(null),
		shelf({}),
		shelf({ type: 'Recent' }),
		shelf({ type: 'script' }),
		shelf({ type: '__proto__' }),
		shelf({ type: 'recent', id: 1 }),
		shelf({ type: 'station' }),
		shelf({ type: 'station', id: 0 }),
		shelf({ type: 'station', id: -1 }),
		shelf({ type: 'station', id: '4' }),
		shelf({ type: 'station', id: 4.5 }),
		shelf({ type: 'station', id: 1e12 }),
		shelf({ type: 'genre', id: Number.MAX_SAFE_INTEGER + 2 }),
		shelf({ type: 'recent', hidden: 'yes' }),
		shelf({ type: 'recent', label: '<img src=x onerror=alert(1)>' }),
		{ version: 1, shelves: [{ type: 'recent' }, { type: 'recent' }] },
		{ version: 1, shelves: [{ type: 'menu', id: 2 }, { type: 'menu', id: 2, hidden: true }] },
		{ version: 1, shelves: Array.from({ length: 25 }, (_, i) => ({ type: 'genre', id: i + 1 })) },
	];
	for (const b of bad) expect(L.layoutProblem(b), JSON.stringify(b)).not.toBeNull();
	expect(L.layoutProblem({ version: 1, shelves: Array.from({ length: 24 }, (_, i) => ({ type: 'genre', id: i + 1 })) })).toBeNull();
	expect(L.layoutProblem({ version: 1, shelves: [] })).toBeNull();
});

test('favourites and playlists are shelves without an id', () => {
	const l = L.add(L.add(L.defaultLayout(), { type: 'favourites' }), { type: 'playlists' });
	expect(L.layoutProblem(L.clean(l))).toBeNull();
	expect(L.shelfProblem({ type: 'favourites', id: 3 })).not.toBeNull();
	expect(L.shelfProblem({ type: 'playlists', id: 1 })).not.toBeNull();
	expect(L.addable(l)).not.toContain('favourites');
	expect(L.SHELF_LABELS.favourites).toBe('Favourites');
});
