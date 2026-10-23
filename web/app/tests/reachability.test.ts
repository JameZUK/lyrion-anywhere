// Reachability (SPEC: controls while the server is out of reach): one state for the whole
// app from the events socket, failed requests, the browser's online flag and this device's
// player socket, with a short threshold so brief blips never flash the UI; and the one
// connection message.
import { ApiError } from '#lib/api/http';
import {
	BUSY_TEXT,
	LOCAL_TEXT,
	ReachabilityTracker,
	UNREACHABLE_AFTER_MS,
	UNREACHABLE_TEXT,
	connectionMessage,
	isNetworkError,
	type Signals,
} from '#lib/state/reachability';

const ok: Signals = { browserOnline: true, events: 'live', requestsFailing: false, deviceDown: false, busy: false };

describe('isNetworkError', () => {
	test('no connection, or a proxy saying the gateway is down', () => {
		expect(isNetworkError(new TypeError('Failed to fetch'))).toBe(true);
		expect(isNetworkError(new ApiError(502, 'bad gateway'))).toBe(true);
		expect(isNetworkError(new ApiError(504, 'gateway timeout'))).toBe(true);
		expect(isNetworkError(new ApiError(503, 'unavailable'))).toBe(true);
		expect(isNetworkError(new ApiError(0, 'none'))).toBe(true);
	});

	test('refusals are not (they are reported as before)', () => {
		expect(isNetworkError(new ApiError(503, 'busy', false, 2, { busy: true }))).toBe(false);
		for (const s of [400, 401, 403, 404, 409, 413, 429, 500]) expect(isNetworkError(new ApiError(s, 'x')), String(s)).toBe(false);
		for (const v of [new Error('x'), 'TypeError', null, undefined, { name: 'TypeError' }, 42]) expect(isNetworkError(v)).toBe(false);
	});
});

describe('ReachabilityTracker', () => {
	test('online while everything answers', () => {
		const t = new ReachabilityTracker();
		expect(t.update(ok, 0)).toEqual({ state: 'online', recheckAt: null });
	});

	test('a blip shorter than the threshold never shows', () => {
		const t = new ReachabilityTracker();
		t.update(ok, 0);
		expect(t.update({ ...ok, events: 'retrying' }, 1000)).toEqual({ state: 'online', recheckAt: 1000 + UNREACHABLE_AFTER_MS });
		expect(t.update({ ...ok, events: 'retrying' }, 1000 + UNREACHABLE_AFTER_MS - 1).state).toBe('online');
		expect(t.update(ok, 1000 + UNREACHABLE_AFTER_MS - 1)).toEqual({ state: 'online', recheckAt: null });
	});

	test('down past the threshold: unreachable, until it answers again', () => {
		const t = new ReachabilityTracker();
		t.update(ok, 0);
		t.update({ ...ok, events: 'retrying' }, 1000);
		expect(t.update({ ...ok, events: 'connecting' }, 1000 + UNREACHABLE_AFTER_MS)).toEqual({ state: 'unreachable', recheckAt: null });
		expect(t.update(ok, 9000).state).toBe('online');
	});

	test('each signal counts on its own', () => {
		for (const s of [{ requestsFailing: true }, { deviceDown: true }, { events: 'retrying' as const }]) {
			const t = new ReachabilityTracker();
			t.update(ok, 0);
			t.update({ ...ok, ...s }, 0);
			expect(t.update({ ...ok, ...s }, UNREACHABLE_AFTER_MS).state, JSON.stringify(s)).toBe('unreachable');
		}
	});

	test('the browser saying it is offline is believed at once', () => {
		const t = new ReachabilityTracker();
		expect(t.update({ ...ok, browserOnline: false }, 0).state).toBe('unreachable');
	});

	test('the first connection is not a lost one', () => {
		const t = new ReachabilityTracker();
		expect(t.update({ ...ok, events: 'connecting' }, 0).state).toBe('online');
		expect(t.update({ ...ok, events: 'connecting' }, 60_000)).toEqual({ state: 'online', recheckAt: null });
		// Once live, connecting again is.
		t.update(ok, 60_000);
		t.update({ ...ok, events: 'connecting' }, 61_000);
		expect(t.update({ ...ok, events: 'connecting' }, 61_000 + UNREACHABLE_AFTER_MS).state).toBe('unreachable');
		// Signed out (stopped): forgotten.
		t.update({ ...ok, events: 'stopped' }, 70_000);
		expect(t.update({ ...ok, events: 'connecting' }, 80_000).state).toBe('online');
	});

	test('busy keeps its own state while the gateway answers', () => {
		const t = new ReachabilityTracker();
		expect(t.update({ ...ok, busy: true }, 0).state).toBe('busy');
		t.update({ ...ok, busy: true, requestsFailing: true }, 10);
		expect(t.update({ ...ok, busy: true, requestsFailing: true }, 10 + UNREACHABLE_AFTER_MS).state).toBe('unreachable');
	});

	test('reset forgets a connection that was going down', () => {
		const t = new ReachabilityTracker();
		t.update({ ...ok, requestsFailing: true }, 0);
		t.reset();
		expect(t.update({ ...ok, requestsFailing: true }, UNREACHABLE_AFTER_MS)).toEqual({ state: 'online', recheckAt: 2 * UNREACHABLE_AFTER_MS });
	});
});

describe('the connection message', () => {
	test('one message, British English, nothing while online', () => {
		expect(connectionMessage('online', true)).toBe('');
		expect(connectionMessage('busy', false)).toBe(BUSY_TEXT);
		expect(BUSY_TEXT).toBe('Your music server is busy. Waiting for it…');
		expect(connectionMessage('unreachable', false)).toBe(UNREACHABLE_TEXT);
		expect(connectionMessage('unreachable', true)).toBe(LOCAL_TEXT);
		expect(LOCAL_TEXT).toMatch(/^Can’t reach your music server\. Playing from this device\.$/);
		expect(connectionMessage('unreachable', false, 'Next track needs your music server.')).toBe('Can’t reach your music server. Next track needs your music server.');
	});
});

describe('the app', () => {
	const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
	const PLAYER = '02:00:00:00:00:01';
	let calls: { player: string; cmd: unknown[] }[];
	let respond: () => Response | Promise<Response>;
	let mod: typeof import('#lib/state/app.svelte');

	beforeAll(async () => {
		mod = await import('#lib/state/app.svelte');
	});

	beforeEach(() => {
		calls = [];
		respond = () => json(200, { result: { mode: 'play' } });
		vi.spyOn(mod.session, 'authed').mockImplementation((fn) => fn('tok'));
		vi.stubGlobal('fetch', async (_u: string, init: RequestInit) => {
			calls.push(JSON.parse(init.body as string));
			return respond();
		});
		mod.app.players = [{ playerid: PLAYER, name: 'Test player' } as never];
		mod.app.playerId = PLAYER;
		mod.app.error = '';
		mod.app.notice = '';
		mod.app.status = 'signed-in';
	});

	afterEach(async () => {
		vi.useRealTimers();
		respond = () => json(200, { result: { players_loop: [{ playerid: PLAYER, name: 'Test player' }] } });
		await mod.app.refresh();
		await new Promise((r) => setTimeout(r, 150));
		mod.app.status = 'unknown';
		vi.unstubAllGlobals();
		vi.restoreAllMocks();
	});

	test('network failures: no error banner; unreachable after the threshold; back on the next success', async () => {
		vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
		respond = () => {
			throw new TypeError('Failed to fetch');
		};
		await mod.app.refresh();
		expect(mod.app.error).toBe('');
		expect(mod.app.reach).toBe('online'); // not yet: it may be a blip
		await vi.advanceTimersByTimeAsync(UNREACHABLE_AFTER_MS);
		expect(mod.app.reach).toBe('unreachable');
		respond = () => json(200, { result: { mode: 'play' } });
		await mod.app.refresh();
		expect(mod.app.reach).toBe('online');
	});

	test('a proxy error page (502) is out of reach too, not an error', async () => {
		respond = () => new Response('<html>502</html>', { status: 502 });
		await mod.app.refresh();
		expect(mod.app.error).toBe('');
	});

	test('real refusals still show', async () => {
		respond = () => json(403, { error: 'command not allowed' });
		await mod.app.refresh();
		expect(mod.app.error).toBe('command not allowed');
	});

	test('while out of reach, commands are not sent at all (and say nothing)', async () => {
		mod.app.setDeviceDown(true);
		vi.stubGlobal('navigator', { onLine: false });
		mod.app.setDeviceDown(false);
		expect(mod.app.reach).toBe('unreachable');
		calls = [];
		await mod.app.send(['pause', '1']);
		await mod.app.queueUp({ album: 1 }, 'play');
		expect(calls).toEqual([]);
		expect(mod.app.error).toBe('');
		expect(mod.app.notice).toBe('');
		vi.stubGlobal('navigator', { onLine: true });
		dispatchEvent(new Event('online'));
		expect(mod.app.reach).toBe('online');
	});

	test('this device’s player socket counts while it is the player', async () => {
		vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
		mod.app.setDeviceDown(true);
		await vi.advanceTimersByTimeAsync(UNREACHABLE_AFTER_MS);
		expect(mod.app.reach).toBe('unreachable');
		mod.app.setDeviceDown(false);
		expect(mod.app.reach).toBe('online');
	});
});
