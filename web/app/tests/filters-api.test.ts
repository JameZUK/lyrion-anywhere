// Smart filters (wave2-design.md §5.6 › The contract): the client-side checks that mirror the
// contract (every field and operator, refused before anything is sent), the request shapes,
// untrusted answers, and the play/shuffle commands.
import { MeApi } from '$lib/api/me';
import {
	DECADES,
	FILTER_LIMITS,
	cleanFilterName,
	cleanSpec,
	filterFrom,
	filterProblem,
	ruleProblem,
	ruleSummary,
	type FilterSpec,
} from '$lib/api/filters';
import { filterPlayCommands, playFilter } from '$lib/filters/play';

type Call = { url: string; method: string; body: unknown };
let calls: Call[] = [];
let reply: { status: number; body: unknown } = { status: 200, body: {} };

beforeEach(() => {
	calls = [];
	reply = { status: 200, body: {} };
	globalThis.fetch = vi.fn(async (url: RequestInfo | URL, init?: RequestInit) => {
		calls.push({ url: String(url), method: init?.method ?? 'GET', body: init?.body ? JSON.parse(String(init.body)) : undefined });
		return new Response(JSON.stringify(reply.body), { status: reply.status, headers: { 'content-type': 'application/json' } });
	}) as typeof fetch;
});

const me = new MeApi((fn) => fn('tok'));
const good = (): FilterSpec => ({
	name: 'Nineties rock',
	rules: [
		{ field: 'genre', in: [3, 4] },
		{ field: 'decade', is: 1990 },
		{ field: 'artist', is: 12, role: 'COMPOSER' },
		{ field: 'compilation', is: false },
	],
	sort: 'yearalbum',
	limit: 200,
});

describe('the contract, checked locally', () => {
	test('a full, valid filter passes', () => {
		expect(filterProblem(good())).toBeNull();
		expect(filterProblem({ ...good(), rules: [{ field: 'compilation', is: true }], sort: 'random', limit: 1 })).toBeNull();
		expect(filterProblem({ ...good(), limit: 500 })).toBeNull();
	});

	test('genre: 1 to 20 ids, positive integers, no repeats, no other keys', () => {
		const g = (v: unknown) => ruleProblem({ field: 'genre', in: v });
		expect(g([1])).toBeNull();
		expect(g(Array.from({ length: 20 }, (_, i) => i + 1))).toBeNull();
		expect(g([])).not.toBeNull();
		expect(g(Array.from({ length: 21 }, (_, i) => i + 1))).not.toBeNull();
		for (const bad of [[0], [-1], [1.5], ['3'], [null], [Number.MAX_SAFE_INTEGER + 1], [[1]], [{ id: 1 }], [1, 1]]) expect(g(bad)).not.toBeNull();
		expect(g('1,2')).not.toBeNull();
		expect(ruleProblem({ field: 'genre', is: [1] })).not.toBeNull();
		expect(ruleProblem({ field: 'genre', in: [1], not: true })).not.toBeNull();
	});

	test('decade: a multiple of ten from 1900 to 2090', () => {
		const d = (v: unknown) => ruleProblem({ field: 'decade', is: v });
		expect(d(1900)).toBeNull();
		expect(d(2090)).toBeNull();
		for (const bad of [1890, 2100, 1995, 1990.5, '1990', null, NaN, Infinity, [1990], { is: 1990 }]) expect(d(bad)).not.toBeNull();
		expect(ruleProblem({ field: 'decade', in: [1990] })).not.toBeNull();
		expect(ruleProblem({ field: 'decade', is: 1990, role: 'ARTIST' })).not.toBeNull();
		expect(DECADES[0]).toBe(1900);
		expect(DECADES.at(-1)).toBe(2090);
		expect(DECADES).toHaveLength(20);
	});

	test('artist: an id, and optionally one of the browse roles', () => {
		const a = (r: Record<string, unknown>) => ruleProblem({ field: 'artist', ...r });
		expect(a({ is: 1 })).toBeNull();
		for (const role of ['ALBUMARTIST', 'ARTIST', 'COMPOSER', 'CONDUCTOR', 'BAND', 'TRACKARTIST']) expect(a({ is: 1, role })).toBeNull();
		for (const bad of [0, -3, 2.5, '12', null, [12]]) expect(a({ is: bad })).not.toBeNull();
		for (const role of ['composer', 'PRODUCER', '', 'ARTIST;DROP', 7, null]) expect(a({ is: 1, role })).not.toBeNull();
		expect(a({ is: 1, name: 'Someone' })).not.toBeNull();
	});

	test('compilation: true or false only', () => {
		expect(ruleProblem({ field: 'compilation', is: true })).toBeNull();
		expect(ruleProblem({ field: 'compilation', is: false })).toBeNull();
		for (const bad of [1, 0, 'true', null, undefined]) expect(ruleProblem({ field: 'compilation', is: bad })).not.toBeNull();
	});

	test('unknown fields, operators and shapes are refused', () => {
		for (const bad of [null, 1, 'genre', [], { field: 'rating', is: 5 }, { field: 'GENRE', in: [1] }, { in: [1] }, { field: ['genre'], in: [1] }]) {
			expect(ruleProblem(bad)).not.toBeNull();
		}
	});

	test('1 to 8 rules, each field once', () => {
		expect(filterProblem({ ...good(), rules: [] })).not.toBeNull();
		expect(filterProblem({ ...good(), rules: [{ field: 'decade', is: 1990 }, { field: 'decade', is: 2000 }] })).toMatch(/once/);
		const nine = Array.from({ length: 9 }, () => ({ field: 'compilation', is: true }));
		expect(filterProblem({ ...good(), rules: nine })).not.toBeNull();
		expect(filterProblem({ ...good(), rules: 'genre' })).not.toBeNull();
	});

	test('the name: 1–64 characters, cleaned like device names', () => {
		expect(cleanFilterName('  Late   night\tjazz  ')).toBe('Late night jazz');
		expect(cleanFilterName('<script>alert(1)</script>')).toBe('scriptalert(1)script');
		expect(cleanFilterName('a‮b​c\u0000d')).toBe('abcd');
		expect(cleanFilterName('Café & Crème')).toBe('Café & Crème');
		expect(cleanFilterName('🎵🎵')).toBe('');
		expect([...cleanFilterName('x'.repeat(100))].length).toBe(64);
		expect(filterProblem({ ...good(), name: '' })).not.toBeNull();
		expect(filterProblem({ ...good(), name: ' padded ' })).not.toBeNull();
		expect(filterProblem({ ...good(), name: 'x'.repeat(65) })).not.toBeNull();
		expect(filterProblem({ ...good(), name: 42 })).not.toBeNull();
	});

	test('sort and limit', () => {
		for (const sort of ['album', 'artistalbum', 'yearalbum', 'new', 'random']) expect(filterProblem({ ...good(), sort })).toBeNull();
		for (const sort of ['played', 'recentlyplayed', 'ALBUM', '', null]) expect(filterProblem({ ...good(), sort })).not.toBeNull();
		for (const limit of [0, 501, 1.5, '50', null, -1, Infinity]) expect(filterProblem({ ...good(), limit })).not.toBeNull();
	});

	test('unknown keys, an id in the body, nesting and oversize bodies are refused', () => {
		expect(filterProblem({ ...good(), id: 3 })).not.toBeNull();
		expect(filterProblem({ ...good(), owner: 'someone' })).not.toBeNull();
		expect(filterProblem({ ...good(), rules: [{ field: 'genre', in: [1], rules: [{ field: 'genre', in: [2] }] }] })).not.toBeNull();
		expect(filterProblem(null)).not.toBeNull();
		expect(filterProblem([good()])).not.toBeNull();
		// The largest possible valid filter still fits in 4 KiB.
		const big: FilterSpec = { ...good(), name: 'x'.repeat(64), rules: [{ field: 'genre', in: Array.from({ length: 20 }, (_, i) => 999_999_999_000 + i) }, ...good().rules.slice(1)] };
		expect(filterProblem(big)).toBeNull();
		expect(new TextEncoder().encode(JSON.stringify(big)).length).toBeLessThanOrEqual(FILTER_LIMITS.maxBytes);
	});

	test('robustness: arbitrary input never throws', () => {
		let seed = 7;
		const rnd = () => ((seed = (seed * 1103515245 + 12345) % 2 ** 31) / 2 ** 31);
		const atoms = [null, undefined, 0, -1, 1990, 2.5, '', 'genre', 'decade', 'artist', 'compilation', true, false, [], {}, [1, 2], 'x'.repeat(5000)];
		const any = (depth: number): unknown => {
			const r = rnd();
			if (depth > 3 || r < 0.5) return atoms[Math.floor(rnd() * atoms.length)];
			if (r < 0.75) return Array.from({ length: Math.floor(rnd() * 4) }, () => any(depth + 1));
			const keys = ['field', 'in', 'is', 'role', 'name', 'rules', 'sort', 'limit', 'id', 'x'];
			return Object.fromEntries(Array.from({ length: Math.floor(rnd() * 5) }, () => [keys[Math.floor(rnd() * keys.length)]!, any(depth + 1)]));
		};
		for (let i = 0; i < 3000; i++) {
			const v = any(0);
			expect(() => filterProblem(v)).not.toThrow();
			expect(() => ruleProblem(v)).not.toThrow();
			expect(() => filterFrom(v)).not.toThrow();
		}
	});
});

describe('the API wrapper', () => {
	test('lists filters, leaving out malformed ones', async () => {
		reply.body = {
			filters: [
				{ id: 1, ...good() },
				{ id: 0, ...good() },
				{ id: 2, ...good(), rules: [] },
				{ id: '3', ...good() },
				{ id: 4, name: 'Odd', rules: [{ field: 'rating', is: 5 }], sort: 'album', limit: 10 },
				'junk',
				{ id: 5, ...good(), extra: 'ignored' },
			],
		};
		const list = await me.filters();
		expect(list.map((f) => f.id)).toEqual([1, 5]);
		expect(list[1]).not.toHaveProperty('extra');
		expect(calls[0]).toMatchObject({ url: '/v1/me/filters', method: 'GET' });
		reply.body = { filters: 'nope' };
		expect(await me.filters()).toEqual([]);
	});

	test('creates with POST and no id; the id comes only from the server', async () => {
		reply = { status: 201, body: { id: 9 } };
		expect(await me.createFilter({ ...good(), name: '  Nineties   rock ' })).toBe(9);
		expect(calls[0]).toMatchObject({ url: '/v1/me/filters', method: 'POST' });
		expect(calls[0]!.body).toEqual(good());
		expect(calls[0]!.body).not.toHaveProperty('id');
		reply = { status: 201, body: { id: 'x' } };
		await expect(me.createFilter(good())).rejects.toThrow();
	});

	test('nothing invalid is ever sent', async () => {
		const bad: unknown[] = [
			{ ...good(), rules: [] },
			{ ...good(), limit: 501 },
			{ ...good(), sort: 'played' },
			{ ...good(), name: '🎵' },
			{ ...good(), rules: [{ field: 'decade', is: 1995 }] },
			{ ...good(), rules: [{ field: 'genre', in: Array.from({ length: 21 }, (_, i) => i + 1) }] },
			{ ...good(), rules: [{ field: 'artist', is: 1, role: 'PRODUCER' }] },
			{ ...good(), rules: [{ field: 'rating', is: 5 }] },
			{ ...good(), rules: null },
		];
		for (const b of bad) {
			await expect(me.createFilter(b as FilterSpec)).rejects.toThrow(RangeError);
			await expect(me.updateFilter(1, b as FilterSpec)).rejects.toThrow(RangeError);
		}
		expect(calls).toHaveLength(0);
	});

	test('replaces with PUT and deletes with DELETE, by a checked id', async () => {
		await me.updateFilter(4, good());
		expect(calls[0]).toMatchObject({ url: '/v1/me/filters/4', method: 'PUT', body: good() });
		await me.deleteFilter(4);
		expect(calls[1]).toMatchObject({ url: '/v1/me/filters/4', method: 'DELETE', body: undefined });
		for (const id of [0, -1, 1.5, NaN, '4/../5' as unknown as number]) {
			await expect(me.updateFilter(id, good())).rejects.toThrow(RangeError);
			await expect(me.deleteFilter(id)).rejects.toThrow(RangeError);
			await expect(me.filterAlbums(id)).rejects.toThrow(RangeError);
		}
		expect(calls).toHaveLength(2);
	});

	test('someone else’s filter: the 404 is passed on', async () => {
		reply = { status: 404, body: { error: 'not found' } };
		await expect(me.deleteFilter(8)).rejects.toThrow();
		await expect(me.updateFilter(8, good())).rejects.toThrow();
	});

	test('albums: paged, n at most 100, ids only', async () => {
		reply.body = { albums: [5, '6', 0, -2, 7.5, 8, null], count: 120 };
		expect(await me.filterAlbums(3, 50, 50)).toEqual({ albums: [5, 8], count: 120 });
		expect(calls[0]!.url).toBe('/v1/me/filters/3/albums?from=50&n=50');
		await me.filterAlbums(3, -5, 1000);
		expect(calls[1]!.url).toBe('/v1/me/filters/3/albums?from=0&n=100');
		reply.body = { albums: [1, 2], count: 'lots' };
		expect((await me.filterAlbums(3)).count).toBe(2);
	});

	test('cleanSpec keeps only the contract’s keys', () => {
		const dirty = { ...good(), extra: 1, rules: [{ field: 'decade', is: 1990, extra: 2 }] } as unknown as FilterSpec;
		expect(cleanSpec(dirty)).toEqual({ ...good(), rules: [{ field: 'decade', is: 1990 }] });
	});

	test('rule summaries', () => {
		expect(ruleSummary({ field: 'decade', is: 1970 })).toBe('1970s');
		expect(ruleSummary({ field: 'compilation', is: true })).toBe('Compilations only');
		expect(ruleSummary({ field: 'genre', in: [1, 2] })).toBe('2 genres');
		expect(ruleSummary({ field: 'genre', in: [1] }, { genre: () => 'Jazz' })).toBe('Jazz');
		expect(ruleSummary({ field: 'artist', is: 1, role: 'COMPOSER' }, { artist: () => 'A. Composer' })).toBe('A. Composer (composer)');
	});
});

describe('play and shuffle', () => {
	test('the first album replaces the queue, the rest are added, up to 50', () => {
		const ids = Array.from({ length: 60 }, (_, i) => i + 1);
		const c = filterPlayCommands(ids, false);
		expect(c).toHaveLength(50);
		expect(c[0]).toEqual(['playlistcontrol', 'cmd:load', 'album_id:1']);
		expect(c[1]).toEqual(['playlistcontrol', 'cmd:add', 'album_id:2']);
		expect(c[49]).toEqual(['playlistcontrol', 'cmd:add', 'album_id:50']);
	});

	test('shuffle turns track shuffle on first', () => {
		const c = filterPlayCommands([4, 5], true);
		expect(c).toEqual([
			['playlist', 'shuffle', 1],
			['playlistcontrol', 'cmd:load', 'album_id:4'],
			['playlistcontrol', 'cmd:add', 'album_id:5'],
		]);
	});

	test('bad or repeated ids are dropped; nothing at all plays nothing', () => {
		expect(filterPlayCommands([0, -1, 2.5, 3, 3] as number[], false)).toEqual([['playlistcontrol', 'cmd:load', 'album_id:3']]);
		expect(filterPlayCommands([], true)).toEqual([]);
	});

	test('playFilter asks for 50 albums, sends in order and stops at a failure', async () => {
		const sent: unknown[] = [];
		const asked: unknown[] = [];
		const deps = {
			albums: async (id: number, from: number, n: number) => (asked.push([id, from, n]), { albums: [7, 8, 9] }),
			send: async (c: (string | number)[]) => {
				sent.push(c);
				if (c.includes('album_id:8')) throw new Error('refused');
			},
		};
		expect(await playFilter(deps, 3, false)).toMatch(/refused/);
		expect(asked).toEqual([[3, 0, 50]]);
		expect(sent).toHaveLength(2);
		expect(await playFilter({ ...deps, albums: async () => ({ albums: [] }) }, 3, false)).toMatch(/No albums/);
		expect(await playFilter({ ...deps, albums: async () => Promise.reject(new Error('gone')) }, 3, false)).toMatch(/gone/);
	});
});
