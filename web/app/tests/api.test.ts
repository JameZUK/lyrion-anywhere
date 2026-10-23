// Session renewal, the RPC/artwork client and the events stream, against fakes.
import { RETRY_MS, SIGNED_OUT_KEY, SessionManager, refused, type AuthApi } from '#lib/api/session';
import { Api, artUrl } from '#lib/api/rpc';
import { ApiError, callJson, CLIENT_HEADER } from '#lib/api/http';
import { EventsClient, eventsBackoff, parseEventMessage } from '#lib/api/events';
import { clientKind, defaultDeviceName, groupKey, LOCK_HOLD_MS, LOCK_WAIT_MS, login, PAGE_ID, refresh, REFRESH_LOCK, REFRESH_TIMEOUT_MS, RefreshTimeout, rememberedDevice, renameDevice, totpSetup, validDeviceName, withRefreshLock, type LockManagerLike } from '#lib/api/auth';
import type { Session } from '#lib/api/auth';

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
	localStorage.clear();
});

/** A promise to settle by hand. */
function deferred<T>() {
	let resolve!: (v: T) => void;
	let reject!: (e: unknown) => void;
	const promise = new Promise<T>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

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

	test('two-step setup sends the account password', async () => {
		const f = vi.fn(async () => json(200, { secret: 'S', uri: 'otpauth://x', qr_svg: '<svg/>' }));
		vi.stubGlobal('fetch', f);
		await totpSetup('t', 'my password');
		const [url, init] = f.mock.calls[0] as unknown as [string, RequestInit];
		expect(url).toBe('/v1/account/totp/setup');
		expect(init.method).toBe('POST');
		expect(JSON.parse(init.body as string)).toEqual({ password: 'my password' });
		expect(init.headers).toMatchObject({ Authorization: 'Bearer t', 'Content-Type': 'application/json' });
	});

	// Regression (review, Web low): a proxy's SPA fallback answered /v1/auth/refresh with 200
	// and index.html, which read as {} and was retried for ever as "can't reach".
	test('a 2xx whose body is not JSON is a bad gateway, not an empty answer', async () => {
		vi.stubGlobal('fetch', async () => new Response('<!doctype html><html></html>', { status: 200, headers: { 'content-type': 'text/html' } }));
		const e = await callJson('/v1/auth/refresh').catch((x) => x);
		expect(e).toBeInstanceOf(ApiError);
		expect(e).toMatchObject({ status: 502 });
		vi.stubGlobal('fetch', async () => new Response('{"truncated', { status: 200, headers: { 'content-type': 'application/json' } }));
		await expect(callJson('/v1/x')).rejects.toMatchObject({ status: 502 });
		// An empty 2xx body (and 204) still read as nothing to say.
		vi.stubGlobal('fetch', async () => new Response('', { status: 200 }));
		expect(await callJson('/v1/x')).toEqual({});
		vi.stubGlobal('fetch', async () => new Response(null, { status: 204 }));
		expect(await callJson('/v1/x')).toBeNull();
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

	// Regression: a phone waking up offline, or the gateway restarting, signed the user out.
	test('a renewal that cannot reach the server keeps the sign-in and retries', async () => {
		vi.useFakeTimers();
		const outcomes: (string | Error)[] = ['t1', new TypeError('Failed to fetch'), new ApiError(502, 'HTTP 502'), 't2'];
		const auth = fakeAuth([]);
		auth.refresh = vi.fn(async () => {
			const o = outcomes.shift();
			if (o instanceof Error) throw o;
			if (!o) throw new ApiError(401, 'expired');
			return sess(o);
		});
		const s = new SessionManager(auth);
		await s.restore();
		expect(await s.renew()).toBe(false);
		expect(s.status).toBe('signed-in');
		expect(s.token).toBe('t1');
		await vi.advanceTimersByTimeAsync(RETRY_MS[0]!); // still 502
		expect(s.status).toBe('signed-in');
		await vi.advanceTimersByTimeAsync(RETRY_MS[1]!);
		expect(s.token).toBe('t2');
	});

	test('restore while offline waits and retries instead of showing the sign-in form', async () => {
		vi.useFakeTimers();
		const outcomes: (string | Error)[] = [new TypeError('Failed to fetch'), 't1'];
		const auth = fakeAuth([]);
		auth.refresh = vi.fn(async () => {
			const o = outcomes.shift();
			if (o instanceof Error) throw o;
			return sess(o!);
		});
		const s = new SessionManager(auth);
		const states: string[] = [];
		s.subscribe((_, st) => states.push(st));
		expect(await s.restore()).toBe(false);
		expect(s.status).toBe('offline');
		await vi.advanceTimersByTimeAsync(RETRY_MS[0]!);
		expect(s.status).toBe('signed-in');
		expect(states).not.toContain('signed-out');
	});

	test('only refusals sign out', () => {
		for (const st of [400, 401, 403]) expect(refused(new ApiError(st, 'x'))).toBe(true);
		for (const st of [408, 429, 500, 502, 503, 504]) expect(refused(new ApiError(st, 'x'))).toBe(false);
		expect(refused(new TypeError('Failed to fetch'))).toBe(false);
		expect(refused(new DOMException('aborted', 'AbortError'))).toBe(false);
	});

	test('sign-out stops pending retries', async () => {
		vi.useFakeTimers();
		const auth = fakeAuth([]);
		auth.refresh = vi.fn(async () => Promise.reject(new TypeError('offline')));
		const s = new SessionManager(auth);
		await s.restore();
		await s.signOut();
		await vi.advanceTimersByTimeAsync(120_000);
		expect(auth.refresh).toHaveBeenCalledTimes(1);
		expect(s.status).toBe('signed-out');
	});

	test('each refresh says why it was asked for (for the gateway log)', async () => {
		vi.useFakeTimers();
		const outcomes: (string | Error)[] = ['t1', 't2', new TypeError('Failed to fetch'), 't3', 't4'];
		const auth = fakeAuth([]);
		const reasons: (string | undefined)[] = [];
		auth.refresh = vi.fn(async (reason?: string) => {
			reasons.push(reason);
			const o = outcomes.shift();
			if (o instanceof Error) throw o;
			return { ...sess(o!), expires_in: 60 };
		});
		const s = new SessionManager(auth);
		await s.restore();
		await s.authed(async (t) => (t === 't1' ? Promise.reject(new ApiError(401, 'x')) : t));
		await s.renew('events');
		await vi.advanceTimersByTimeAsync(RETRY_MS[0]!);
		await vi.advanceTimersByTimeAsync(30_000);
		expect(reasons).toEqual(['load', '401', 'events', 'retry', 'timer']);
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

describe('sign-out holds', () => {
	// Regression: a renewal in flight when the user signed out applied its new token
	// afterwards, silently signing them back in.
	test('a renewal in flight at sign-out does not sign back in', async () => {
		const auth = fakeAuth(['t1']);
		const s = new SessionManager(auth);
		await s.restore();
		const pending = deferred<Session>();
		auth.refresh = vi.fn(() => pending.promise);
		const renewing = s.renew();
		await Promise.resolve();
		await Promise.resolve();
		expect(auth.refresh).toHaveBeenCalledTimes(1);
		await s.signOut();
		pending.resolve(sess('t-late'));
		expect(await renewing).toBe(false);
		expect(s.status).toBe('signed-out');
		expect(s.token).toBe('');
	});

	test('a renewal failing offline after sign-out schedules no retries', async () => {
		vi.useFakeTimers();
		const auth = fakeAuth(['t1']);
		const s = new SessionManager(auth);
		await s.restore();
		const pending = deferred<Session>();
		auth.refresh = vi.fn(() => pending.promise);
		const renewing = s.renew();
		await Promise.resolve();
		await s.signOut();
		pending.reject(new TypeError('Failed to fetch'));
		expect(await renewing).toBe(false);
		await vi.advanceTimersByTimeAsync(10 * 60_000);
		expect(auth.refresh).toHaveBeenCalledTimes(1);
		expect(s.status).toBe('signed-out');
	});

	test('a restore in flight at sign-out does not sign in', async () => {
		const auth = fakeAuth([]);
		const pending = deferred<Session>();
		auth.refresh = vi.fn(() => pending.promise);
		const s = new SessionManager(auth);
		const restoring = s.restore();
		await s.signOut();
		pending.resolve(sess('t-late'));
		expect(await restoring).toBe(false);
		expect(s.status).toBe('signed-out');
	});

	test('a retry pending from before sign-out is cancelled', async () => {
		vi.useFakeTimers();
		const auth = fakeAuth(['t1']);
		const s = new SessionManager(auth);
		await s.restore();
		auth.refresh = vi.fn(async () => Promise.reject(new TypeError('offline')));
		expect(await s.renew()).toBe(false); // schedules a retry
		await s.signOut();
		auth.refresh = vi.fn(async () => sess('t2'));
		await vi.advanceTimersByTimeAsync(10 * 60_000);
		expect(auth.refresh).not.toHaveBeenCalled();
		expect(s.status).toBe('signed-out');
	});

	test('a sign-out that reached the server leaves no flag', async () => {
		const s = new SessionManager(fakeAuth(['t1']));
		await s.restore();
		await s.signOut();
		expect(localStorage.getItem(SIGNED_OUT_KEY)).toBeNull();
	});

	// Regression: signing out offline, then reloading, restored the session from the cookie.
	test('a sign-out that could not reach the server holds after a reload, and is finished then', async () => {
		const auth = fakeAuth(['t1']);
		auth.logout = vi.fn(async () => Promise.reject(new TypeError('offline')));
		const s = new SessionManager(auth);
		await s.restore();
		await s.signOut();
		expect(localStorage.getItem(SIGNED_OUT_KEY)).toBe('1');

		// The next page load: the cookie still works, but the user must not be signed in.
		const auth2 = fakeAuth(['t-cookie']);
		const s2 = new SessionManager(auth2);
		const states: string[] = [];
		s2.subscribe((_, st) => states.push(st));
		expect(await s2.restore()).toBe(false);
		expect(s2.status).toBe('signed-out');
		await vi.waitFor(() => expect(auth2.logout).toHaveBeenCalledWith('t-cookie'));
		await vi.waitFor(() => expect(localStorage.getItem(SIGNED_OUT_KEY)).toBeNull());
		expect(states).not.toContain('signed-in');
		expect(s2.token).toBe('');
	});

	test('if finishing the sign-out fails offline, the flag stays for the next load', async () => {
		localStorage.setItem(SIGNED_OUT_KEY, '1');
		const auth = fakeAuth([]);
		auth.refresh = vi.fn(async () => Promise.reject(new TypeError('offline')));
		const s = new SessionManager(auth);
		expect(await s.restore()).toBe(false);
		expect(s.status).toBe('signed-out');
		await vi.waitFor(() => expect(auth.refresh).toHaveBeenCalled());
		await Promise.resolve();
		expect(localStorage.getItem(SIGNED_OUT_KEY)).toBe('1');
	});

	test('a refused cookie finishes the pending sign-out (nothing left to revoke)', async () => {
		localStorage.setItem(SIGNED_OUT_KEY, '1');
		const auth = fakeAuth([]); // refresh: 401
		const s = new SessionManager(auth);
		await s.restore();
		await vi.waitFor(() => expect(localStorage.getItem(SIGNED_OUT_KEY)).toBeNull());
		expect(auth.logout).not.toHaveBeenCalled();
	});

	test('signing in explicitly clears the flag', async () => {
		localStorage.setItem(SIGNED_OUT_KEY, '1');
		const auth = fakeAuth([]);
		auth.refresh = vi.fn(() => new Promise<Session>(() => {})); // the background clean-up hangs
		const s = new SessionManager(auth);
		await s.restore();
		await s.signIn('alice', 'pw');
		expect(s.status).toBe('signed-in');
		expect(localStorage.getItem(SIGNED_OUT_KEY)).toBeNull();
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
		const f = vi.fn(async () => new Response(new TextEncoder().encode('png'), { headers: { 'content-type': 'image/png' } }));
		vi.stubGlobal('fetch', f);
		const created: string[] = [];
		vi.stubGlobal('URL', Object.assign(URL, { createObjectURL: () => (created.push('blob:x'), 'blob:x'), revokeObjectURL: vi.fn() }));
		const u = await (await api()).art('music/12/cover_300x300');
		expect(u).toBe('blob:x');
		const [url, init] = f.mock.calls[0] as unknown as [string, RequestInit];
		expect(url).toBe('/v1/art/music/12/cover_300x300');
		expect(url).not.toContain('t1');
		expect(init.headers).toMatchObject({ Authorization: 'Bearer t1' });
		// Never a stale copy forced from the browser cache, which isn't kept apart per account.
		expect(init.cache).toBe('default');
	});

	// Regression: Api.art trusted its path, so "imageproxy/x/image/.%2E/.%2E/.%2E/.%2E/me"
	// fetched /v1/me with the bearer token.
	test('art never fetches outside /v1/art/, whatever the encoding', async () => {
		const f = vi.fn(async () => new Response('x', { headers: { 'content-type': 'image/png' } }));
		vi.stubGlobal('fetch', f);
		const a = await api();
		const bad = [
			'imageproxy/x/image/.%2E/.%2E/.%2E/.%2E/me',
			'imageproxy/x/image/.%2e/.%2e/.%2e/.%2e/me',
			'imageproxy/x/image/%2E%2E/%2e%2E/%2E%2e/%2e%2e/me',
			'imageproxy/x/image/../../../../me',
			'music/1/cover/../../../../me',
			'music\\..\\..\\..\\me',
			'music/1/%5C..%5C..%5Cme',
			'..',
			'.%2E',
			'%2E%2E/%2E%2E/me',
			'music/1/cover?x=/../../me',
			'music/1/cover#../../me',
			'//evil.example.com/x',
			'',
		];
		for (const p of bad) {
			await expect(a.art(p), p).rejects.toMatchObject({ status: 400 });
		}
		expect(f).not.toHaveBeenCalled();
		// Encoded slashes inside a segment are not traversal: they stay under /v1/art/.
		expect(artUrl('imageproxy/https%3A%2F%2Fcdn.example.com%2Fa.jpg/image_300x300')).toBe(
			'/v1/art/imageproxy/https%3A%2F%2Fcdn.example.com%2Fa.jpg/image_300x300',
		);
		expect(artUrl('music/12/cover_300x300')).toBe('/v1/art/music/12/cover_300x300');
	});

	test('double-encoded dots are left encoded (no second decoding), so stay under /v1/art/', () => {
		const u = artUrl('music/%252E%252E/%252E%252E/me');
		expect(u === null || u.startsWith('/v1/art/music/')).toBe(true);
	});

	// Covers kept with offline downloads use the same checks as on-screen art.
	test('art as a Blob (for downloads): same path checks, images only', async () => {
		const f = vi.fn(async () => new Response(new TextEncoder().encode('png'), { headers: { 'content-type': 'image/png' } }));
		vi.stubGlobal('fetch', f);
		const a = await api();
		const b = await a.artBlob('music/12/cover_300x300');
		expect(b.type).toBe('image/png');
		expect((f.mock.calls[0] as unknown as [string])[0]).toBe('/v1/art/music/12/cover_300x300');
		for (const p of ['..', 'music/1/cover/../../../../me', '//evil.example.com/x']) await expect(a.artBlob(p), p).rejects.toMatchObject({ status: 400 });
		expect(f).toHaveBeenCalledTimes(1);
		vi.stubGlobal('fetch', async () => new Response('<svg onload=x>', { headers: { 'content-type': 'text/html' } }));
		await expect(a.artBlob('music/1/cover')).rejects.toMatchObject({ status: 415 });
	});

	test('non-image art responses are refused', async () => {
		vi.stubGlobal('fetch', async () => new Response('<svg onload=x>', { headers: { 'content-type': 'text/html' } }));
		await expect((await api()).art('music/1/cover')).rejects.toMatchObject({ status: 415 });
	});

	test('the art cache is bounded and revokes evicted URLs', async () => {
		vi.stubGlobal('fetch', async () => new Response(new TextEncoder().encode('x'), { headers: { 'content-type': 'image/jpeg' } }));
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

	// Regression: the LRU revoked blob: URLs that images on screen were still showing.
	test('art held on screen is never revoked; it goes once released', async () => {
		vi.stubGlobal('fetch', async () => new Response(new TextEncoder().encode('x'), { headers: { 'content-type': 'image/jpeg' } }));
		let n = 0;
		const revoke = vi.fn();
		vi.stubGlobal('URL', Object.assign(URL, { createObjectURL: () => `blob:${++n}`, revokeObjectURL: revoke }));
		const a = await api(); // cache size 2
		const r1 = a.retainArt('music/1/cover');
		await a.art('music/1/cover');
		const r2 = a.retainArt('music/2/cover');
		await a.art('music/2/cover');
		const r3 = a.retainArt('music/3/cover');
		await a.art('music/3/cover'); // over the size, but every one is on screen
		await a.art('music/4/cover'); // not held: may go, but not while it is the one just asked for
		await Promise.resolve();
		expect(revoke).not.toHaveBeenCalled();
		// Asked again, a held one comes from the cache (same URL, no new fetch).
		expect(await a.art('music/1/cover')).toBe('blob:1');
		// Released: the oldest unheld ones go until the cache fits.
		r2();
		r2(); // twice does nothing more
		await Promise.resolve();
		await Promise.resolve();
		expect(revoke.mock.calls.map((c) => c[0]).sort()).toEqual(['blob:2', 'blob:4']);
		expect(await a.art('music/3/cover')).toBe('blob:3');
		r1();
		r3();
	});

	test('two views of the same art: it stays until both let go', async () => {
		vi.stubGlobal('fetch', async () => new Response(new TextEncoder().encode('x'), { headers: { 'content-type': 'image/jpeg' } }));
		let n = 0;
		const revoke = vi.fn();
		vi.stubGlobal('URL', Object.assign(URL, { createObjectURL: () => `blob:${++n}`, revokeObjectURL: revoke }));
		const a = await api();
		const one = a.retainArt('music/1/cover');
		const two = a.retainArt('music/1/cover');
		await a.art('music/1/cover');
		one();
		await a.art('music/2/cover');
		await a.art('music/3/cover');
		await Promise.resolve();
		expect(revoke).not.toHaveBeenCalledWith('blob:1');
		two();
		await a.art('music/4/cover'); // now it is the oldest nobody holds
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

	// Regression: a renewal that failed because the server was unreachable stopped the events
	// stream for good, though the user stayed signed in.
	test('a renewal that fails offline keeps retrying; the stream resumes once it succeeds', async () => {
		vi.useFakeTimers();
		FakeWs.all = [];
		const outcomes: (string | Error)[] = ['t1', new TypeError('Failed to fetch'), 't2'];
		const auth = fakeAuth([]);
		auth.refresh = vi.fn(async () => {
			const o = outcomes.shift();
			if (o instanceof Error) throw o;
			if (!o) throw new TypeError('offline');
			return sess(o);
		});
		const s = new SessionManager(auth);
		await s.restore();
		const states: string[] = [];
		let resyncs = 0;
		const c = new EventsClient(s, { onEvent: () => {}, onResync: () => resyncs++, onState: (x) => states.push(x) }, (u) => new FakeWs(u) as unknown as WebSocket);
		c.start();
		await FakeWs.all[0]!.onclose!({ code: 1008, reason: 'unauthorised' });
		expect(s.status).toBe('signed-in');
		expect(states.at(-1)).toBe('retrying');
		// The session retries the renewal; the events client reconnects meanwhile.
		await vi.advanceTimersByTimeAsync(RETRY_MS[0]! + 2000);
		expect(s.token).toBe('t2');
		const last = FakeWs.all.at(-1)!;
		expect(FakeWs.all.length).toBeGreaterThan(1);
		last.onopen!();
		expect(last.sent).toEqual(['t2']);
		last.onmessage!({ data: '{"subscribed":true}' });
		expect(states.at(-1)).toBe('live');
		expect(resyncs).toBe(1);
		c.stop();
	});

	test('an unauthorised close after the session was signed out stops', async () => {
		FakeWs.all = [];
		const s = new SessionManager(fakeAuth(['t1'])); // the renewal is refused
		await s.restore();
		const states: string[] = [];
		const c = new EventsClient(s, { onEvent: () => {}, onResync: () => {}, onState: (x) => states.push(x) }, (u) => new FakeWs(u) as unknown as WebSocket);
		c.start();
		await FakeWs.all[0]!.onclose!({ code: 1008, reason: 'unauthorised' });
		expect(s.status).toBe('signed-out');
		expect(states.at(-1)).toBe('stopped');
	});
});

describe('auth helpers', () => {
	test('device names', () => {
		expect(defaultDeviceName('Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0')).toBe('Firefox on Linux');
		expect(defaultDeviceName('Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) AppleWebKit/605.1.15 Version/26.0 Mobile/15E148 Safari/604.1')).toBe('Safari on iOS');
		expect(defaultDeviceName('')).toBe('Browser on unknown');
		// iPads ask for desktop sites and say "Macintosh"; touch gives them away.
		const ipad = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15';
		expect(defaultDeviceName(ipad, 5)).toBe('Safari on iPadOS');
		expect(defaultDeviceName(ipad, 0)).toBe('Safari on macOS');
		expect(defaultDeviceName('Mozilla/5.0 (iPad; CPU OS 18_7 like Mac OS X) AppleWebKit/605.1.15 CriOS/140.0 Mobile/15E148 Safari/604.1')).toBe('Chrome on iPadOS');
		expect(defaultDeviceName('Mozilla/5.0 (Linux; Android 15; Pixel 8) AppleWebKit/537.36 Chrome/140.0 Mobile Safari/537.36')).toBe('Chrome on Android');
	});

	test('client kind: installed (standalone) or a browser tab', () => {
		expect(clientKind(() => false, undefined)).toBe('browser');
		expect(clientKind((q) => q === '(display-mode: standalone)', undefined)).toBe('pwa');
		expect(clientKind((q) => q === '(display-mode: minimal-ui)', undefined)).toBe('pwa');
		expect(clientKind(() => false, true)).toBe('pwa'); // iOS home-screen apps
		expect(clientKind(() => false, 'yes')).toBe('browser');
		expect(clientKind(() => { throw new Error('no matchMedia'); }, undefined)).toBe('browser');
	});

	test('device names for renaming', () => {
		expect(validDeviceName('  Kitchen iPad ')).toBe('Kitchen iPad');
		expect(validDeviceName('é'.repeat(64))).toBe('é'.repeat(64));
		for (const bad of ['', '   ', 'x'.repeat(65), 'a\nb', 'a\u0000b', 'a\u007fb']) expect(validDeviceName(bad)).toBeNull();
	});

	test('sign-in and renewal tell the gateway what kind of client this is', async () => {
		const calls: RequestInit[] = [];
		vi.stubGlobal('fetch', vi.fn(async (_u: string, init: RequestInit) => {
			calls.push(init);
			return json(200, sess('t'));
		}));
		vi.stubGlobal('matchMedia', (q: string) => ({ matches: q === '(display-mode: standalone)' }));
		await login('alice', 'pw');
		await refresh(undefined);
		for (const init of calls) {
			const body = JSON.parse(String(init.body));
			expect(body.client_kind).toBe('pwa');
			expect(typeof body.client_platform).toBe('string');
			expect(body.client_platform.length).toBeLessThanOrEqual(48);
			expect((init.headers as Record<string, string>)['Content-Type']).toBe('application/json');
			expect((init.headers as Record<string, string>)['X-Anywhere-Client']).toBe('1');
		}
		expect(Object.keys(JSON.parse(String(calls[1]!.body))).sort()).toEqual(['client_kind', 'client_platform']);
	});

	test('renaming this device', async () => {
		const f = vi.fn(async () => json(200, { id: 3, name: 'Kitchen iPad' }));
		vi.stubGlobal('fetch', f);
		expect(await renameDevice('tok', 'Kitchen iPad')).toEqual({ id: 3, name: 'Kitchen iPad' });
		const [url, init] = f.mock.calls[0] as unknown as [string, RequestInit];
		expect(url).toBe('/v1/devices/self');
		expect(init.method).toBe('PATCH');
		expect(init.body).toBe('{"name":"Kitchen iPad"}');
		const h = init.headers as Record<string, string>;
		expect(h.Authorization).toBe('Bearer tok');
		expect(h['X-Anywhere-Client']).toBe('1');
		vi.stubGlobal('fetch', vi.fn(async () => json(400, { error: 'another of this user\'s devices already has that name' })));
		await expect(renameDevice('tok', 'Phone')).rejects.toThrow(/already has that name/);
	});

	test('a rename shows at once', async () => {
		const s = new SessionManager(fakeAuth(['t1']), undefined);
		await s.restore();
		const names: string[] = [];
		s.subscribe((cur) => cur && names.push(cur.device.name));
		s.setDeviceName('Kitchen iPad');
		expect(s.current?.device.name).toBe('Kitchen iPad');
		expect(s.current?.device.id).toBe(3);
		expect(names.at(-1)).toBe('Kitchen iPad');
		await s.signOut();
		s.setDeviceName('x');
		expect(s.current).toBeNull();
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

/** A fake `navigator.locks`: grants each named lock to one holder at a time, in order. */
function fakeLocks() {
	const tails = new Map<string, Promise<unknown>>();
	const names: string[] = [];
	return {
		names,
		request<T>(name: string, cb: () => Promise<T>): Promise<T> {
			names.push(name);
			const prev = tails.get(name) ?? Promise.resolve();
			const run = prev.then(cb, cb);
			tails.set(
				name,
				run.catch(() => {}),
			);
			return run;
		},
	};
}

describe('cross-tab refresh lock', () => {
	/** A fetch whose refresh responses are released by hand, tracking concurrency. */
	function fakeRefreshServer() {
		let active = 0;
		let maxActive = 0;
		let n = 0;
		const pending: ReturnType<typeof deferred<void>>[] = [];
		const f = vi.fn(async () => {
			active++;
			maxActive = Math.max(maxActive, active);
			const gate = deferred<void>();
			pending.push(gate);
			await gate.promise;
			active--;
			n++;
			return json(200, { ...sess(`t${n}`), device: { id: 7, name: 'x' } });
		});
		return { f, pending, max: () => maxActive };
	}

	test('refreshes from two tabs run one after the other under the named lock', async () => {
		const server = fakeRefreshServer();
		vi.stubGlobal('fetch', server.f);
		const locks = fakeLocks();
		vi.stubGlobal('navigator', { locks, userAgent: '' });
		const a = refresh();
		const b = refresh();
		await vi.waitFor(() => expect(server.pending.length).toBe(1));
		// The second tab waits for the first to finish before presenting the cookie.
		await new Promise((r) => setTimeout(r, 10));
		expect(server.f).toHaveBeenCalledTimes(1);
		server.pending[0]!.resolve();
		expect((await a).access_token).toBe('t1');
		await vi.waitFor(() => expect(server.pending.length).toBe(2));
		server.pending[1]!.resolve();
		expect((await b).access_token).toBe('t2');
		expect(server.max()).toBe(1);
		expect(locks.names).toEqual([REFRESH_LOCK, REFRESH_LOCK]);
		expect(REFRESH_LOCK).toBe('anywhere-refresh');
	});

	test('a refresh sends its reason and this page\'s id', async () => {
		const server = fakeRefreshServer();
		vi.stubGlobal('fetch', server.f);
		vi.stubGlobal('navigator', { userAgent: '' });
		const p = refresh('timer');
		await vi.waitFor(() => expect(server.pending.length).toBe(1));
		server.pending[0]!.resolve();
		await p;
		const h = new Headers((server.f.mock.calls[0] as unknown as [string, RequestInit])[1].headers);
		expect(h.get('x-anywhere-refresh-reason')).toBe('timer');
		expect(h.get('x-anywhere-page')).toBe(PAGE_ID);
		// Only what the gateway logs: lower-case letters and digits, short.
		expect(PAGE_ID).toMatch(/^[a-z0-9]{1,16}$/);
	});

	test('a failed refresh releases the lock for the next one', async () => {
		const locks = fakeLocks();
		const first = withRefreshLock(async () => {
			throw new ApiError(401, 'expired');
		}, locks);
		await expect(first).rejects.toThrow();
		await expect(withRefreshLock(async () => 'ok', locks)).resolves.toBe('ok');
	});

	test('without Web Locks, refresh simply runs', async () => {
		const server = fakeRefreshServer();
		vi.stubGlobal('fetch', server.f);
		vi.stubGlobal('navigator', { userAgent: '' });
		const p = refresh();
		await vi.waitFor(() => expect(server.pending.length).toBe(1));
		server.pending[0]!.resolve();
		expect((await p).access_token).toBe('t1');
		// A broken lock manager is ignored too.
		expect(await withRefreshLock(async () => 5, { request: undefined } as unknown as LockManagerLike)).toBe(5);
		expect(await withRefreshLock(async () => 6, undefined)).toBe(6);
	});

	// Regression (review, Web low): a refresh that never answered held the cross-tab lock for
	// ever, so every tab, the events socket and the player worker waited behind it.
	describe('a hung refresh never holds the lock for ever', () => {
		beforeEach(() => vi.useFakeTimers());
		afterEach(() => vi.useRealTimers());

		test('a request that never answers is abandoned, and the next refresh gets the lock', async () => {
			let calls = 0;
			vi.stubGlobal(
				'fetch',
				vi.fn((_u: string, init: RequestInit) => {
					calls++;
					if (calls === 2) return Promise.resolve(json(200, { ...sess('t2'), device: { id: 7, name: 'x' } }));
					return new Promise((_, rej) => init.signal!.addEventListener('abort', () => rej(new DOMException('aborted', 'AbortError'))));
				}),
			);
			const locks = fakeLocks();
			vi.stubGlobal('navigator', { locks, userAgent: '' });
			const hung = refresh('timer').catch((e) => e);
			const next = refresh('401');
			await vi.advanceTimersByTimeAsync(REFRESH_TIMEOUT_MS + 1);
			const e = await hung;
			expect(e).toBeInstanceOf(RefreshTimeout);
			// Not a refusal: the session keeps its sign-in and tries again.
			expect(refused(e)).toBe(false);
			expect((await next).access_token).toBe('t2');
		});

		test('a holder whose work hangs lets go after the hold limit', async () => {
			const locks = fakeLocks();
			const stuck = withRefreshLock(() => new Promise<never>(() => {}), locks).catch((e) => e);
			const after = withRefreshLock(async () => 'ok', locks);
			await vi.advanceTimersByTimeAsync(LOCK_HOLD_MS + 1);
			expect(await stuck).toBeInstanceOf(RefreshTimeout);
			await expect(after).resolves.toBe('ok');
		});

		test('waiting behind another tab that never lets go gives up, and then never runs', async () => {
			const locks = fakeLocks();
			// Another tab (an older copy of the app, say) holds the lock and never lets go.
			void locks.request(REFRESH_LOCK, () => new Promise<never>(() => {}));
			const ran = vi.fn(async () => 'late');
			const waiting = withRefreshLock(ran, locks).catch((e) => e);
			await vi.advanceTimersByTimeAsync(LOCK_WAIT_MS + 1);
			expect(await waiting).toBeInstanceOf(RefreshTimeout);
			expect(ran).not.toHaveBeenCalled();
		});
	});
});

describe('limiter', () => {
	test('never runs more than the limit at once, and runs everything', async () => {
		const { Limiter } = await import('#lib/api/rpc');
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
		const { Limiter } = await import('#lib/api/rpc');
		const l = new Limiter(1);
		await expect(l.run(async () => Promise.reject(new Error('x')))).rejects.toThrow('x');
		await expect(l.run(async () => 'ok')).resolves.toBe('ok');
	});
});
