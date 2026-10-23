// The app state on a page loaded while the music server is busy or out of reach (2026-10-14
// fixes): Now playing keeps the last state seen rather than "Nothing to control yet", and a
// device someone was signed in on comes back signed in (provisionally) rather than to the
// bare "can't reach" page. Against the real app state, with the network faked.
import { PROFILE_KEY } from '#lib/api/session';
import { LAST_KNOWN_KEY } from '#lib/state/last-known';

const P = '02:00:00:00:00:01';
const NOW = { mode: 'play', time: 98, duration: 200, playlist_cur_index: 2, playlist_tracks: 8, playlist_loop: [{ id: 5, title: 'Night Ferry' }] };
const json = (status: number, body: unknown, headers: Record<string, string> = {}) =>
	new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', ...headers } });
const busy503 = () => json(503, { error: 'music server is busy', busy: true }, { 'Retry-After': '2' });
const session = { access_token: 't1', expires_in: 900, user: { username: 'u', role: 'listener' }, device: { id: 1, name: 'Test browser' } };

/** The network: how the refresh and the RPC calls answer. */
const net = { refresh: () => json(200, session), rpc: (): Response | Promise<Response> => busy503() };

class NoSocket {
	onopen: (() => void) | null = null;
	onclose: ((e: { code: number; reason: string }) => void) | null = null;
	onmessage: (() => void) | null = null;
	send() {}
	close() {}
}

beforeAll(() => {
	vi.stubGlobal('WebSocket', NoSocket);
	vi.stubGlobal('fetch', async (u: string) => {
		if (String(u).includes('/v1/auth/refresh')) return net.refresh();
		if (String(u).includes('/v1/rpc')) return net.rpc();
		throw new TypeError('Failed to fetch');
	});
});
afterAll(() => vi.unstubAllGlobals());

async function freshApp() {
	vi.resetModules();
	return await import('#lib/state/app.svelte');
}
const settle = () => new Promise((r) => setTimeout(r, 50));

beforeEach(() => {
	localStorage.clear();
	sessionStorage.clear();
	localStorage.setItem('anywhere.defaultPlayer.u', JSON.stringify({ kind: 'player', id: P }));
	net.refresh = () => json(200, session);
	net.rpc = () => busy503();
});

describe('a page loaded while the music server is busy', () => {
	test('Now playing shows the last state seen, not "Nothing to control yet"', async () => {
		localStorage.setItem(LAST_KNOWN_KEY, JSON.stringify({ user: 'u', player: { playerid: P, name: 'Lounge' }, now: NOW }));
		const { app, session: s } = await freshApp();
		await s.restore();
		await settle();
		expect(app.status).toBe('signed-in');
		expect(app.reach).toBe('busy');
		expect(app.current?.name).toBe('Lounge');
		expect(app.now?.playlist_loop?.[0]?.title).toBe('Night Ferry');
		expect(app.elapsed).toBe(98);
		// Where it is now isn't known: the position stays put (no pretend progress).
		await new Promise((r) => setTimeout(r, 600));
		expect(app.elapsed).toBe(98);
	});

	test('fresh answers replace it as soon as the server gives them', async () => {
		localStorage.setItem(LAST_KNOWN_KEY, JSON.stringify({ user: 'u', player: { playerid: P, name: 'Lounge' }, now: NOW }));
		net.rpc = () => json(200, { result: { players_loop: [{ playerid: P, name: 'Lounge' }], mode: 'pause', time: 5, playlist_loop: [{ id: 6, title: 'Paper Moons' }] } });
		const { app, session: s } = await freshApp();
		await s.restore();
		await settle();
		await app.refresh();
		expect(app.now?.playlist_loop?.[0]?.title).toBe('Paper Moons');
		expect(app.elapsed).toBe(5);
	});

	test("another user's last state is never shown", async () => {
		localStorage.setItem(LAST_KNOWN_KEY, JSON.stringify({ user: 'someone-else', player: { playerid: P, name: 'Lounge' }, now: NOW }));
		const { app, session: s } = await freshApp();
		await s.restore();
		await settle();
		expect(app.current).toBeUndefined();
		expect(app.now).toBeNull();
	});

	test('what is playing is noted for the next load, and forgotten at sign-out', async () => {
		net.rpc = () => json(200, { result: { players_loop: [{ playerid: P, name: 'Lounge' }], ...NOW } });
		const { app, session: s } = await freshApp();
		await s.restore();
		await settle();
		await app.refresh();
		app.rememberNow();
		expect(JSON.parse(localStorage.getItem(LAST_KNOWN_KEY) ?? '{}')).toMatchObject({ user: 'u', player: { playerid: P }, now: { playlist_loop: [{ title: 'Night Ferry' }] } });
		await s.signOut();
		expect(localStorage.getItem(LAST_KNOWN_KEY)).toBeNull();
	});
});

describe('a page loaded while the server is out of reach', () => {
	test('someone was signed in here: the app comes back as them, not the "can\'t reach" page', async () => {
		localStorage.setItem(PROFILE_KEY, JSON.stringify({ user: session.user, device: session.device }));
		localStorage.setItem(LAST_KNOWN_KEY, JSON.stringify({ user: 'u', player: { playerid: P, name: 'Lounge' }, now: NOW }));
		net.refresh = () => {
			throw new TypeError('Failed to fetch');
		};
		const { app, session: s } = await freshApp();
		await s.restore();
		await settle();
		expect(app.status).toBe('signed-in');
		expect(app.provisional).toBe(true);
		expect(app.now?.playlist_loop?.[0]?.title).toBe('Night Ferry');
		await vi.waitFor(() => expect(app.reach).toBe('unreachable'), { timeout: 5000 });
	});

	test('nobody signed in here: offline mode', async () => {
		net.refresh = () => {
			throw new TypeError('Failed to fetch');
		};
		const { app, session: s } = await freshApp();
		await s.restore();
		expect(app.status).toBe('offline');
		expect(app.provisional).toBe(false);
	});
});
