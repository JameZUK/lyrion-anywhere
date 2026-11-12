// Review (Web low): after an outage the controls stayed disabled for up to ~36 s once the
// server was back, because the events socket waited out its backoff. The app nudges it (see
// EventsClient.nudge) when the browser is online again, when the page is shown again, and on
// the first request that gets through while it is retrying. The real app, fake network.
import { NUDGE_FLOOR_MS } from '#lib/api/events';

const P = '02:00:00:00:00:01';
const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
const SESSION = { access_token: 't1', expires_in: 900, user: { username: 'u', role: 'listener' }, device: { id: 1, name: 'x' } };

class FakeSocket {
	static all: FakeSocket[] = [];
	onopen: (() => void) | null = null;
	onclose: ((e: { code: number; reason: string }) => void | Promise<void>) | null = null;
	onmessage: ((m: { data: unknown }) => void) | null = null;
	constructor(public url: string) {
		FakeSocket.all.push(this);
	}
	send() {}
	close() {}
}

let rpcOk = true;
beforeAll(() => {
	vi.stubGlobal('WebSocket', FakeSocket);
	vi.stubGlobal('fetch', async (u: string) => {
		if (String(u).includes('/v1/auth/refresh')) return json(200, SESSION);
		if (String(u).includes('/v1/rpc')) {
			if (!rpcOk) throw new TypeError('Failed to fetch');
			return json(200, { result: { players_loop: [{ playerid: P, name: 'Lounge' }], mode: 'pause' } });
		}
		throw new TypeError('Failed to fetch');
	});
});
afterAll(() => vi.unstubAllGlobals());
afterEach(() => vi.useRealTimers());

/** A fresh app, signed in, whose events socket has dropped a few times (a long backoff now). */
async function afterAnOutage() {
	vi.resetModules();
	FakeSocket.all = [];
	rpcOk = true;
	localStorage.clear();
	const mod = await import('#lib/state/app.svelte');
	await mod.session.restore();
	await vi.advanceTimersByTimeAsync(10);
	expect(FakeSocket.all.length).toBeGreaterThan(0);
	for (let i = 0; i < 6; i++) {
		await FakeSocket.all.at(-1)!.onclose!({ code: 1006, reason: '' });
		await vi.advanceTimersByTimeAsync(40_000); // past the longest backoff
	}
	// Requests fail too from here, so the app's own polling nudges nothing meanwhile.
	rpcOk = false;
	await FakeSocket.all.at(-1)!.onclose!({ code: 1006, reason: '' });
	expect(mod.app.events).toBe('retrying');
	return { mod, sockets: FakeSocket.all.length };
}

test('back online: the events socket reconnects at once, not at the end of its backoff', async () => {
	vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date'] });
	const { sockets } = await afterAnOutage();
	await vi.advanceTimersByTimeAsync(NUDGE_FLOOR_MS); // the floor since the last attempt
	expect(FakeSocket.all.length).toBe(sockets);
	dispatchEvent(new Event('online'));
	await vi.advanceTimersByTimeAsync(1);
	expect(FakeSocket.all.length).toBe(sockets + 1);
});

test('a request that gets through while it retries brings the socket back too', async () => {
	vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date'] });
	const { mod, sockets } = await afterAnOutage();
	await vi.advanceTimersByTimeAsync(NUDGE_FLOOR_MS);
	await mod.app.loadPlayers(); // still out of reach: nothing
	await vi.advanceTimersByTimeAsync(1);
	expect(FakeSocket.all.length).toBe(sockets);
	rpcOk = true;
	await mod.app.loadPlayers();
	await vi.advanceTimersByTimeAsync(1);
	expect(FakeSocket.all.length).toBe(sockets + 1);
});

test('the page shown again: the same', async () => {
	vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date'] });
	const { sockets } = await afterAnOutage();
	await vi.advanceTimersByTimeAsync(NUDGE_FLOOR_MS);
	Object.defineProperty(document, 'visibilityState', { value: 'visible', configurable: true });
	document.dispatchEvent(new Event('visibilitychange'));
	await vi.advanceTimersByTimeAsync(1);
	expect(FakeSocket.all.length).toBe(sockets + 1);
});
