// The user's own data on the gateway: request shapes, checks before sending, and errors.
import { MeApi, pinId } from '#lib/api/me';
import { ApiError } from '#lib/api/http';

type Call = { url: string; method: string; headers: Record<string, string>; body: unknown };
let calls: Call[] = [];
let reply: { status: number; body: unknown } = { status: 200, body: {} };

beforeEach(() => {
	calls = [];
	reply = { status: 200, body: {} };
	globalThis.fetch = vi.fn(async (url: RequestInfo | URL, init?: RequestInit) => {
		calls.push({
			url: String(url),
			method: init?.method ?? 'GET',
			headers: init?.headers as Record<string, string>,
			body: init?.body ? JSON.parse(String(init.body)) : undefined,
		});
		return new Response(JSON.stringify(reply.body), { status: reply.status, headers: { 'content-type': 'application/json' } });
	}) as typeof fetch;
});

const me = new MeApi((fn) => fn('tok'));
const PLAYER = '02:4c:41:00:00:01';

test('history: paged by cursor, with the bearer token and the client header', async () => {
	reply.body = { enabled: true, retention_days: 396, items: [], next: '1790000000-4' };
	const page = await me.history(20);
	expect(page.next).toBe('1790000000-4');
	expect(calls[0]).toMatchObject({ url: '/v1/me/history?limit=20', method: 'GET' });
	expect(calls[0]!.headers.Authorization).toBe('Bearer tok');
	expect(calls[0]!.headers['X-Anywhere-Client']).toBe('1');
	await me.history(500, '1790000000-4');
	expect(calls[1]!.url).toBe('/v1/me/history?limit=200&before=1790000000-4');
	expect(() => me.history(10, '1;DROP')).toThrow(RangeError);
	expect(() => me.history(10, '../x')).toThrow(RangeError);
});

test('history: switch off and clear', async () => {
	reply.body = { enabled: false };
	await me.setHistory(false);
	expect(calls[0]).toMatchObject({ url: '/v1/me/history', method: 'PATCH', body: { enabled: false } });
	reply.body = { removed: 3 };
	expect((await me.clearHistory()).removed).toBe(3);
	expect(calls[1]).toMatchObject({ url: '/v1/me/history', method: 'DELETE', body: undefined });
});

test('shelves return clean album ids only', async () => {
	reply.body = { shelf: 'recent', albums: [7, '8', -1, 0, 1.5, 9] };
	expect(await me.shelf('recent', 100)).toEqual([7, 9]);
	expect(calls[0]!.url).toBe('/v1/me/shelves/recent?limit=50');
	await expect(me.shelf('../history' as 'recent')).rejects.toThrow(RangeError);
});

test('Home: get, save (only version and shelves) and reset', async () => {
	reply.body = { version: 1, shelves: [{ type: 'recent' }], default: true };
	expect((await me.home()).default).toBe(true);
	await me.saveHome({ version: 1, shelves: [{ type: 'most' }], default: true });
	expect(calls[1]).toMatchObject({ url: '/v1/me/home', method: 'PUT', body: { version: 1, shelves: [{ type: 'most' }] } });
	expect(calls[1]!.body).not.toHaveProperty('default');
	await me.resetHome();
	expect(calls[2]).toMatchObject({ url: '/v1/me/home', method: 'DELETE' });
});

test('stations: create, delete and next, with ids and players checked first', async () => {
	reply.body = { stations: [{ id: 1, name: 'Fresh', kind: 'fresh', spec: { kind: 'fresh' }, broken: null }] };
	expect((await me.stations())[0]!.kind).toBe('fresh');
	await me.createStation('Jazz', { kind: 'genre', genres: [3] });
	expect(calls[1]).toMatchObject({ url: '/v1/me/stations', method: 'POST', body: { name: 'Jazz', spec: { kind: 'genre', genres: [3] } } });
	await me.createStation('Mix', { kind: 'menu', steps: [['trackinfo', 'items', 0, 100, 'track_id:9', 'menu:1']], play: ['x', 'y'] }, 'AA:BB:CC:00:00:01');
	expect((calls[2]!.body as { player: string }).player).toBe('aa:bb:cc:00:00:01');
	await me.deleteStation(4);
	expect(calls[3]).toMatchObject({ url: '/v1/me/stations/4', method: 'DELETE' });
	reply.body = { mode: 'tracks', tracks: [1, 2], refill: true };
	await me.next(4, { player: PLAYER, start: true, count: 99, exclude: [5, -1, 2.5, 6] });
	expect(calls[4]).toMatchObject({ url: '/v1/me/stations/4/next', body: { player: PLAYER, start: true, count: 50, exclude: [5, 6] } });
	for (const bad of [0, -1, 1.5, NaN, '4' as unknown as number, 2 ** 60]) {
		expect(() => pinId(bad)).toThrow(RangeError);
		expect(() => me.deleteStation(bad)).toThrow(RangeError);
	}
	expect(() => me.next(4, { player: '../x' })).toThrow(RangeError);
	expect(calls).toHaveLength(5);
});

test('pinned menus: create, open and delete', async () => {
	reply.body = { id: 3, name: 'My Music', steps: [], broken: null };
	await me.createMenu('My Music', [['menu', 0, 100, 'direct:1']], PLAYER);
	expect(calls[0]).toMatchObject({ url: '/v1/me/menus', method: 'POST', body: { name: 'My Music', player: PLAYER } });
	reply.body = { result: { item_loop: [{ text: 'X' }] } };
	expect(await me.openMenu(3, PLAYER)).toEqual({ item_loop: [{ text: 'X' }] });
	expect(calls[1]).toMatchObject({ url: '/v1/me/menus/3/open', body: { player: PLAYER } });
	await me.deleteMenu(3);
	expect(calls[2]).toMatchObject({ url: '/v1/me/menus/3', method: 'DELETE' });
});

test('refusals come back as ApiError with the gateway reason', async () => {
	reply = { status: 422, body: { error: 'this menu item is no longer offered' } };
	const err = await me.createStation('x', { kind: 'fresh' }).catch((e) => e);
	expect(err).toBeInstanceOf(ApiError);
	expect(err.status).toBe(422);
	expect(err.message).toBe('this menu item is no longer offered');
	reply = { status: 409, body: { error: 'this plugin is for admins only', broken: true } };
	await expect(me.next(1, { player: PLAYER })).rejects.toMatchObject({ status: 409 });
});

test('checked queue edits: done, changed, and an older gateway remembered', async () => {
	const fresh = new MeApi((fn) => fn('tok'));
	reply = { status: 200, body: { done: true } };
	expect(await fresh.queueEdit(PLAYER.toUpperCase(), { op: 'remove', index: 3, track: 42 })).toBe('done');
	expect(calls[0]).toMatchObject({ url: `/v1/players/${encodeURIComponent(PLAYER)}/queue/edit`, method: 'POST', body: { op: 'remove', index: 3, track: 42 } });
	reply = { status: 409, body: { error: 'the queue changed' } };
	expect(await fresh.queueEdit(PLAYER, { op: 'move', index: 3, to: 1, track: 42 })).toBe('changed');
	reply = { status: 500, body: { error: 'boom' } };
	await expect(fresh.queueEdit(PLAYER, { op: 'remove', index: 3, track: 42 })).rejects.toBeInstanceOf(ApiError);
	reply = { status: 404, body: {} };
	expect(await fresh.queueEdit(PLAYER, { op: 'remove', index: 3, track: 42 })).toBe('unsupported');
	const n = calls.length;
	expect(await fresh.queueEdit(PLAYER, { op: 'remove', index: 3, track: 42 })).toBe('unsupported');
	expect(calls.length).toBe(n); // not asked again
	await expect(fresh.queueEdit('not a player', { op: 'remove', index: 3, track: 42 })).rejects.toThrow();
});
