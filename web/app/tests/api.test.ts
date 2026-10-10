// Session renewal, the RPC/artwork client and the events stream, against fakes.
import { SessionManager, type AuthApi } from '$lib/api/session';
import { Api } from '$lib/api/rpc';
import { ApiError, callJson, CLIENT_HEADER } from '$lib/api/http';
import { EventsClient, eventsBackoff, parseEventMessage } from '$lib/api/events';
import { defaultDeviceName, groupKey, rememberedDevice } from '$lib/api/auth';
import type { Session } from '$lib/api/auth';

const sess = (token: string): Session => ({
	access_token: token,
	expires_in: 900,
	user: { username: 'alice', role: 'listener' },
	device: { id: 3, name: 'Firefox on Linux' },
});

function fakeAuth(tokens: string[]): AuthApi & { refreshes: number } {
	const api = {
		refreshes: 0,
		login: vi.fn(async () => sess('t-login')),
		refresh: vi.fn(async () => {
			api.refreshes++;
			const t = tokens.shift();
			if (!t) throw new ApiError(401, 'expired');
			return sess(t);
		}),
		logout: vi.fn(async () => {}),
	};
	return api;
}

function json(status: number, body: unknown, headers: Record<string, string> = {}) {
	return new Response(status === 204 ? null : JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json', ...headers },
	});
}

afterEach(() => {
	vi.unstubAllGlobals();
	vi.useRealTimers();
});

describe('http', () => {
	test('every call carries the CSRF header and same-origin credentials only', async () => {
		const f = vi.fn(async () => json(200, { ok: 1 }));
		vi.stubGlobal('fetch', f);
		await callJson('/v1/x', { headers: { Authorization: 'Bearer t' } });
		const [, init] = f.mock.calls[0] as unknown as [string, RequestInit];
		expect(init.credentials).toBe('same-origin');
		expect(init.headers).toMatchObject({ ...CLIENT_HEADER, Authorization: 'Bearer t' });
	});

	test('errors carry status, message and the two-step flag', async () => {
		vi.stubGlobal('fetch', async () => json(401, { error: 'bad', totp_required: true }));
		const e = await callJson('/v1/auth/login').catch((x) => x);
		expect(e).toBeInstanceOf(ApiError);
		expect(e).toMatchObject({ status: 401, message: 'bad', totpRequired: true });
	});

	test('a non-JSON error body still gives a clean error', async () => {
		vi.stubGlobal('fetch', async () => new Response('<html>502</html>', { status: 502 }));
		await expect(callJson('/v1/rpc')).rejects.toMatchObject({ status: 502, message: 'HTTP 502' });
	});
});

describe('session', () => {
	test('restore signs in silently with the refresh cookie, or reports signed out', async () => {
		const s = new SessionManager(fakeAuth(['t1']));
		expect(await s.restore()).toBe(true);
		expect(s.token).toBe('t1');
		const s2 = new SessionManager(fakeAuth([]));
		expect(await s2.restore()).toBe(false);
		expect(s2.status).toBe('signed-out');
	});

	test('a 401 renews the token once and retries with the new one', async () => {
		const auth = fakeAuth(['t1', 't2']);
		const s = new SessionManager(auth);
		await s.restore();
		const seen: string[] = [];
		const r = await s.authed(async (t) => {
			seen.push(t);
			if (t === 't1') throw new ApiError(401, 'expired');
			return 'ok';
		});
		expect(r).toBe('ok');
		expect(seen).toEqual(['t1', 't2']);
	});

	test('a second 401 after renewal is not retried again', async () => {
		const s = new SessionManager(fakeAuth(['t1', 't2', 't3']));
		await s.restore();
		let calls = 0;
		await expect(
			s.authed(async () => {
				calls++;
				throw new ApiError(401, 'revoked');
			}),
		).rejects.toThrow('revoked');
		expect(calls).toBe(2);
	});

	test('a failed renewal signs out', async () => {
		const s = new SessionManager(fakeAuth(['t1']));
		await s.restore();
		const states: string[] = [];
		s.subscribe((_, st) => states.push(st));
		await expect(s.authed(async () => Promise.reject(new ApiError(401, 'x')))).rejects.toThrow();
		expect(s.status).toBe('signed-out');
		expect(s.token).toBe('');
		expect(states.at(-1)).toBe('signed-out');
	});

	test('other errors are not retried', async () => {
		const auth = fakeAuth(['t1', 't2']);
		const s = new SessionManager(auth);
		await s.restore();
		await expect(s.authed(async () => Promise.reject(new ApiError(403, 'policy')))).rejects.toThrow('policy');
		expect(auth.refreshes).toBe(1);
	});

	test('concurrent 401s share one refresh (refresh tokens are single use)', async () => {
		const auth = fakeAuth(['t1', 't2', 't3']);
		const s = new SessionManager(auth);
		await s.restore();
		const call = (t: string) => (t === 't1' ? Promise.reject(new ApiError(401, 'x')) : Promise.resolve(t));
		const results = await Promise.all([s.authed(call), s.authed(call), s.authed(call)]);
		expect(results).toEqual(['t2', 't2', 't2']);
		expect(auth.refreshes).toBe(2); // restore + one renewal
	});

	test('calls before sign-in are refused without a request', async () => {
		const s = new SessionManager(fakeAuth([]));
		const fn = vi.fn();
		await expect(s.authed(fn)).rejects.toMatchObject({ status: 401 });
		expect(fn).not.toHaveBeenCalled();
	});

	test('the token is renewed before it expires', async () => {
		vi.useFakeTimers();
		const auth = fakeAuth(['t1', 't2']);
		const s = new SessionManager(auth);
		await s.restore();
		await vi.advanceTimersByTimeAsync(10 * 60 * 1000 + 1000);
		expect(s.token).toBe('t2');
	});

	test('sign-out forgets the token even if the server call fails', async () => {
		const auth = fakeAuth(['t1']);
		auth.logout = vi.fn(async () => Promise.reject(new Error('offline')));
		const s = new SessionManager(auth);
		await s.restore();
		await s.signOut();
		expect(s.token).toBe('');
		expect(auth.logout).toHaveBeenCalledWith('t1');
	});
});

describe('rpc and art', () => {
	async function api(tokens = ['t1', 't2']) {
		const s = new SessionManager(fakeAuth(tokens));
		await s.restore();
		return new Api(s, 2);
	}

	test('rpc sends player and command with the bearer token', async () => {
		const f = vi.fn(async () => json(200, { result: { count: 1 } }));
		vi.stubGlobal('fetch', f);
		const r = await (await api()).rpc('aa:bb:cc:dd:ee:ff', ['status', '-', 1]);
		expect(r).toEqual({ count: 1 });
		const [url, init] = f.mock.calls[0] as unknown as [string, RequestInit];
		expect(url).toBe('/v1/rpc');
		expect(JSON.parse(init.body as string)).toEqual({ player: 'aa:bb:cc:dd:ee:ff', cmd: ['status', '-', 1] });
		expect(init.headers).toMatchObject({ Authorization: 'Bearer t1', 'X-Anywhere-Client': '1' });
	});

	test('a policy refusal surfaces as a 403 error', async () => {
		vi.stubGlobal('fetch', async () => json(403, { error: 'command "pref" not allowed' }));
		await expect((await api()).rpc('', ['pref'])).rejects.toMatchObject({ status: 403 });
	});

	test('art becomes a blob URL; tokens never go in the URL', async () => {
		const f = vi.fn(async () => new Response(new Blob(['png']), { headers: { 'content-type': 'image/png' } }));
		vi.stubGlobal('fetch', f);
		const created: string[] = [];
		vi.stubGlobal('URL', Object.assign(URL, { createObjectURL: () => (created.push('blob:x'), 'blob:x'), revokeObjectURL: vi.fn() }));
		const u = await (await api()).art('music/12/cover_300x300');
		expect(u).toBe('blob:x');
		const [url, init] = f.mock.calls[0] as unknown as [string, RequestInit];
		expect(url).toBe('/v1/art/music/12/cover_300x300');
		expect(url).not.toContain('t1');
		expect(init.headers).toMatchObject({ Authorization: 'Bearer t1' });
	});

	test('non-image art responses are refused', async () => {
		vi.stubGlobal('fetch', async () => new Response('<svg onload=x>', { headers: { 'content-type': 'text/html' } }));
		await expect((await api()).art('music/1/cover')).rejects.toMatchObject({ status: 415 });
	});

	test('the art cache is bounded and revokes evicted URLs', async () => {
		vi.stubGlobal('fetch', async () => new Response(new Blob(['x']), { headers: { 'content-type': 'image/jpeg' } }));
		let n = 0;
		const revoke = vi.fn();
		vi.stubGlobal('URL', Object.assign(URL, { createObjectURL: () => `blob:${++n}`, revokeObjectURL: revoke }));
		const a = await api();
		await a.art('music/1/cover');
		await a.art('music/2/cover');
		await a.art('music/3/cover'); // cache size 2: the first is evicted
		await Promise.resolve();
		expect(revoke).toHaveBeenCalledWith('blob:1');
	});
});

describe('events', () => {
	test('messages are parsed strictly', () => {
		expect(parseEventMessage('{"subscribed":true}')).toBe('subscribed');
		expect(parseEventMessage('{"resync":true}')).toBe('resync');
		expect(parseEventMessage('{"player":"aa:bb:cc:dd:ee:ff","event":["pause","1"]}')).toEqual({
			player: 'aa:bb:cc:dd:ee:ff',
			event: ['pause', '1'],
		});
		for (const bad of ['', 'nope', '[]', 'null', '{"event":"x"}', '{"player":1,"event":[]}', '{"player":null,"event":[1]}', 42, 'x'.repeat(70000)]) {
			expect(parseEventMessage(bad)).toBeNull();
		}
	});

	test('backoff grows and is capped', () => {
		expect(eventsBackoff(0, () => 0.5)).toBe(1000);
		expect(eventsBackoff(3, () => 0.5)).toBe(8000);
		expect(eventsBackoff(20, () => 1)).toBe(36000);
	});

	class FakeWs {
		static all: FakeWs[] = [];
		sent: string[] = [];
		onopen?: () => void;
		onmessage?: (m: { data: unknown }) => void;
		onclose?: (e: { code: number; reason: string }) => void;
		constructor(public url: string) {
			FakeWs.all.push(this);
		}
		send(s: string) {
			this.sent.push(s);
		}
		close() {}
	}

	test('sends the token first, then delivers events; resyncs on subscribe', async () => {
		FakeWs.all = [];
		const s = new SessionManager(fakeAuth(['t1']));
		await s.restore();
		const events: unknown[] = [];
		let resyncs = 0;
		const c = new EventsClient(s, { onEvent: (e) => events.push(e), onResync: () => resyncs++ }, (u) => new FakeWs(u) as unknown as WebSocket);
		c.start();
		const ws = FakeWs.all[0]!;
		expect(ws.url).toMatch(/^wss?:\/\/.*\/v1\/events$/);
		ws.onopen!();
		expect(ws.sent).toEqual(['t1']);
		ws.onmessage!({ data: '{"subscribed":true}' });
		ws.onmessage!({ data: '{"player":"aa:bb:cc:dd:ee:ff","event":["play"]}' });
		ws.onmessage!({ data: '<script>' });
		expect(resyncs).toBe(1);
		expect(events).toHaveLength(1);
		c.stop();
	});

	test('an unauthorised close renews the token and reconnects; a revoked one stops', async () => {
		vi.useFakeTimers();
		FakeWs.all = [];
		const s = new SessionManager(fakeAuth(['t1', 't2']));
		await s.restore();
		const states: string[] = [];
		const c = new EventsClient(s, { onEvent: () => {}, onResync: () => {}, onState: (x) => states.push(x) }, (u) => new FakeWs(u) as unknown as WebSocket);
		c.start();
		await FakeWs.all[0]!.onclose!({ code: 1008, reason: 'unauthorised' });
		await vi.advanceTimersByTimeAsync(2000);
		expect(FakeWs.all).toHaveLength(2);
		FakeWs.all[1]!.onopen!();
		expect(FakeWs.all[1]!.sent).toEqual(['t2']);
		await FakeWs.all[1]!.onclose!({ code: 1008, reason: 'access revoked' });
		await vi.advanceTimersByTimeAsync(60_000);
		expect(FakeWs.all).toHaveLength(2);
		expect(states.at(-1)).toBe('stopped');
	});
});

describe('auth helpers', () => {
	test('device names', () => {
		expect(defaultDeviceName('Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0')).toBe('Firefox on Linux');
		expect(defaultDeviceName('Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) AppleWebKit/605.1.15 Version/26.0 Mobile/15E148 Safari/604.1')).toBe('Safari on iOS');
		expect(defaultDeviceName('')).toBe('Browser on unknown');
	});

	test('remembered device ids are validated', () => {
		const st = (v: string | null) => ({ getItem: () => v });
		expect(rememberedDevice(st('12'))).toBe(12);
		for (const bad of ['0', '-1', 'x', '1e400', null, '1.5']) expect(rememberedDevice(st(bad))).toBeUndefined();
		expect(rememberedDevice({ getItem: () => { throw new Error('blocked'); } })).toBeUndefined();
	});

	test('keys are grouped for reading', () => {
		expect(groupKey('ABCDEFGHIJ')).toBe('ABCD EFGH IJ');
	});
});

describe('limiter', () => {
	test('never runs more than the limit at once, and runs everything', async () => {
		const { Limiter } = await import('$lib/api/rpc');
		const l = new Limiter(3);
		let active = 0;
		let peak = 0;
		const done: number[] = [];
		await Promise.all(
			Array.from({ length: 20 }, (_, i) =>
				l.run(async () => {
					active++;
					peak = Math.max(peak, active);
					await new Promise((r) => setTimeout(r, 1 + (i % 3)));
					active--;
					done.push(i);
				}),
			),
		);
		expect(peak).toBe(3);
		expect(done).toHaveLength(20);
	});

	test('a failing task frees its slot', async () => {
		const { Limiter } = await import('$lib/api/rpc');
		const l = new Limiter(1);
		await expect(l.run(async () => Promise.reject(new Error('x')))).rejects.toThrow('x');
		await expect(l.run(async () => 'ok')).resolves.toBe('ok');
	});
});
