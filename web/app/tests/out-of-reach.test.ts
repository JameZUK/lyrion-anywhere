// The app while the server is out of reach (2026-10-14 fixes): a route typed into the address
// bar opens in the app instead of reloading it; a page loaded while the server is away comes
// back signed in (provisionally, with no token) when someone was signed in here; the last
// now-playing state is kept for the next load; and controls that need the server say why
// they are disabled.
import { readFileSync } from 'node:fs';
import { goto } from '$app/navigation';
import { outsideRoute, routeOutsideHashChanges } from '#lib/nav';
import { PROFILE_KEY, RETRY_MS, SessionManager, parseProfile, type AuthApi } from '#lib/api/session';
import { ApiError } from '#lib/api/http';
import type { Session } from '#lib/api/auth';
import { LAST_KNOWN_KEY, LAST_KNOWN_MAX, forgetLastKnown, loadLastKnown, saveLastKnown } from '#lib/state/last-known';
import { CONNECTION_ID, explain, needsServer } from '#lib/state/reachability';

const ROUTER = { 'sveltekit:history': 5, 'sveltekit:navigation': 5 };

/** Strings of every kind, for robustness checks. */
function* junk(n = 400): Generator<string> {
	const parts = ['#', '/', '//', '?', '%', '..', 'a', '{', '}', '"', ':', '[', ']', 'null', 'true', '1', '\u0000', '‮', 'é', ' ', '\\', 'javascript:', 'http://x.example'];
	let seed = 7;
	const rnd = () => (seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648;
	for (let i = 0; i < n; i++) {
		let s = '';
		const len = Math.floor(rnd() * 12);
		for (let j = 0; j < len; j++) s += parts[Math.floor(rnd() * parts.length)];
		yield s;
	}
}

afterEach(() => {
	vi.useRealTimers();
	localStorage.clear();
});

describe('a route typed into the address bar', () => {
	test('a hash change from outside the router goes to that route in the app', () => {
		expect(outsideRoute(null, '#/rooms', '#/album/7')).toBe('#/rooms');
		expect(outsideRoute(null, '#/', '#/album/7')).toBe('#/');
		expect(outsideRoute(null, '', '#/album/7')).toBe('#/');
		expect(outsideRoute(undefined, '#/search?q=x', '#/')).toBe('#/search?q=x');
	});

	test("the router's own history entries (Back and Forward) are left to it", () => {
		expect(outsideRoute(ROUTER, '#/rooms', '#/album/7')).toBeNull();
	});

	test('the same page (an anchor on it) is left alone', () => {
		expect(outsideRoute(null, '#/album/7', '#/album/7')).toBeNull();
		expect(outsideRoute(null, '#/album/7#top', '#/album/7')).toBeNull();
	});

	test('anything that is not an app route is refused', () => {
		for (const h of ['#javascript:alert(1)', '#//evil.example/x', '#http://evil.example', '#top', '#%2F%2Fevil.example']) {
			expect(outsideRoute(null, h, '#/')).toBeNull();
		}
	});

	test('never throws, and only ever gives an in-app link', () => {
		for (const s of junk()) {
			for (const state of [null, undefined, 1, 'x', {}, [], ROUTER]) {
				const r = outsideRoute(state, s, '#/');
				if (r !== null) {
					expect(r.startsWith('#/')).toBe(true);
					expect(r.startsWith('#//')).toBe(false);
				}
			}
		}
	});

	test('the listener steps back over the stray entry, then routes there without a reload', async () => {
		const later = vi.fn();
		const back = vi.spyOn(history, 'back').mockImplementation(() => {});
		vi.mocked(goto).mockClear();
		const stop = routeOutsideHashChanges(() => '#/album/7');
		addEventListener('popstate', later); // SvelteKit's own listener (it would reload)
		try {
			history.replaceState(null, '', '#/rooms');
			dispatchEvent(new PopStateEvent('popstate', { state: null }));
			expect(later).not.toHaveBeenCalled();
			expect(back).toHaveBeenCalledTimes(1);
			expect(goto).not.toHaveBeenCalled();
			// Back on the page shown (the router's entry): now the route asked for.
			dispatchEvent(new PopStateEvent('popstate', { state: ROUTER }));
			// Only once the router has seen the event too (it cancels navigations begun before).
			expect(goto).not.toHaveBeenCalled();
			await new Promise((r) => setTimeout(r, 0));
			expect(goto).toHaveBeenCalledWith('#/rooms');
			expect(later).toHaveBeenCalledTimes(1); // the router still sees its own entry
			// Back and Forward through the router's entries are untouched.
			dispatchEvent(new PopStateEvent('popstate', { state: ROUTER }));
			expect(later).toHaveBeenCalledTimes(2);
			expect(back).toHaveBeenCalledTimes(1);
		} finally {
			stop();
			removeEventListener('popstate', later);
			back.mockRestore();
		}
	});

	test('once stopped, it no longer intercepts', () => {
		const later = vi.fn();
		const stop = routeOutsideHashChanges(() => '#/');
		stop();
		addEventListener('popstate', later);
		dispatchEvent(new PopStateEvent('popstate', { state: null }));
		removeEventListener('popstate', later);
		expect(later).toHaveBeenCalledTimes(1);
	});
});

const sess = (token: string, user = 'alice'): Session => ({
	access_token: token,
	expires_in: 900,
	user: { username: user, role: 'listener' },
	device: { id: 3, name: 'Firefox on Linux', mac: '02:00:00:00:00:03' },
});

function scripted(outcomes: (string | Error)[]): AuthApi & { refresh: ReturnType<typeof vi.fn> } {
	return {
		login: vi.fn(async () => sess('t-login')),
		refresh: vi.fn(async () => {
			const o = outcomes.shift();
			if (o instanceof Error) throw o;
			if (!o) throw new TypeError('Failed to fetch');
			return sess(o);
		}),
		logout: vi.fn(async () => {}),
	};
}

/** Sign in once (as on an earlier page load), so this device remembers who. */
async function signedInBefore() {
	const s = new SessionManager(scripted(['t0']));
	await s.restore();
	expect(s.status).toBe('signed-in');
}

describe('a page loaded while the server is out of reach', () => {
	test('comes back signed in as whoever was signed in here, provisionally and with no token', async () => {
		vi.useFakeTimers();
		await signedInBefore();
		const s = new SessionManager(scripted([new TypeError('Failed to fetch'), 't1']));
		const states: string[] = [];
		s.subscribe((_, st) => states.push(st));
		expect(await s.restore()).toBe(false);
		expect(s.status).toBe('signed-in');
		expect(s.provisional).toBe(true);
		expect(s.current?.user).toEqual({ username: 'alice', role: 'listener' });
		expect(s.current?.device.mac).toBe('02:00:00:00:00:03');
		expect(s.token).toBe('');
		expect(states).not.toContain('offline');
		// The server answers: confirmed, with a real token.
		await vi.advanceTimersByTimeAsync(RETRY_MS[0]!);
		expect(s.provisional).toBe(false);
		expect(s.token).toBe('t1');
	});

	test('no request ever goes with an empty token while provisional', async () => {
		vi.useFakeTimers();
		await signedInBefore();
		const s = new SessionManager(scripted([new TypeError('Failed to fetch'), new TypeError('Failed to fetch'), 't1']));
		await s.restore();
		const fn = vi.fn(async (t: string) => t);
		// Still out of reach: refused as a network failure (so the app shows the connection status).
		await expect(s.authed(fn)).rejects.toBeInstanceOf(TypeError);
		expect(fn).not.toHaveBeenCalled();
		// In reach again: the call gets a real token.
		expect(await s.authed(fn)).toBe('t1');
		expect(fn).toHaveBeenCalledWith('t1');
		expect(fn.mock.calls.every(([t]) => t !== '')).toBe(true);
	});

	test('the server refusing the sign-in ends it: signed out, and the device forgets who', async () => {
		vi.useFakeTimers();
		await signedInBefore();
		const s = new SessionManager(scripted([new TypeError('Failed to fetch'), new ApiError(401, 'expired')]));
		await s.restore();
		expect(s.provisional).toBe(true);
		await vi.advanceTimersByTimeAsync(RETRY_MS[0]!);
		expect(s.status).toBe('signed-out');
		expect(s.provisional).toBe(false);
		expect(s.current).toBeNull();
		expect(localStorage.getItem(PROFILE_KEY)).toBeNull();
	});

	test('a refusal on a call while provisional signs out too', async () => {
		await signedInBefore();
		const s = new SessionManager(scripted([new TypeError('Failed to fetch'), new ApiError(401, 'expired')]));
		await s.restore();
		await expect(s.authed(async (t) => t)).rejects.toMatchObject({ status: 401 });
		expect(s.status).toBe('signed-out');
	});

	test('nobody signed in here: offline mode, as before', async () => {
		const s = new SessionManager(scripted([new TypeError('Failed to fetch')]));
		await s.restore();
		expect(s.status).toBe('offline');
		expect(s.current).toBeNull();
	});

	test('after signing out, a load while out of reach does not sign back in', async () => {
		const first = new SessionManager(scripted(['t0']));
		await first.restore();
		await first.signOut();
		expect(localStorage.getItem(PROFILE_KEY)).toBeNull();
		const s = new SessionManager(scripted([new TypeError('Failed to fetch')]));
		await s.restore();
		expect(s.status).not.toBe('signed-in');
	});

	test('a sign-out that has not reached the server still wins over the stored profile', async () => {
		await signedInBefore();
		localStorage.setItem('anywhere.signedOut', '1');
		const s = new SessionManager(scripted([new TypeError('Failed to fetch')]));
		await s.restore();
		expect(s.status).toBe('signed-out');
	});

	test('the stored profile never holds a token', async () => {
		await signedInBefore();
		const raw = localStorage.getItem(PROFILE_KEY) ?? '';
		expect(raw).not.toMatch(/t0|access_token|token/);
		expect(JSON.parse(raw)).toEqual({ user: { username: 'alice', role: 'listener' }, device: { id: 3, name: 'Firefox on Linux', mac: '02:00:00:00:00:03' } });
	});

	test('a tampered or malformed profile is no profile at all', async () => {
		const good = { user: { username: 'alice', role: 'listener' }, device: { id: 3, name: 'x' } };
		const bad: unknown[] = [
			'not json',
			'[]',
			'null',
			'"alice"',
			{ ...good, user: { username: 'alice', role: 'root' } },
			{ ...good, user: { username: '', role: 'admin' } },
			{ ...good, user: { username: 'a'.repeat(201), role: 'admin' } },
			{ ...good, user: { username: 5, role: 'admin' } },
			{ ...good, device: { id: -1, name: 'x' } },
			{ ...good, device: { id: 1.5, name: 'x' } },
			{ ...good, device: { id: '3', name: 'x' } },
			{ ...good, device: { id: 3, name: 7 } },
			{ user: good.user },
			{ device: good.device },
			{ ...good, pad: 'x'.repeat(4000) },
		];
		for (const b of bad) {
			const raw = typeof b === 'string' ? b : JSON.stringify(b);
			expect(parseProfile(raw)).toBeNull();
			localStorage.setItem(PROFILE_KEY, raw);
			const s = new SessionManager(scripted([new TypeError('Failed to fetch')]));
			await s.restore();
			expect(s.status).toBe('offline');
		}
		expect(parseProfile(JSON.stringify(good))).toEqual(good);
		// A player id that doesn't look like one is dropped, not trusted.
		expect(parseProfile(JSON.stringify({ ...good, device: { id: 3, name: 'x', mac: '<b>' } }))?.device).toEqual({ id: 3, name: 'x' });
		// Only the known fields are kept.
		expect(parseProfile(JSON.stringify({ ...good, access_token: 'x', user: { ...good.user, extra: 1 } }))).toEqual(good);
	});

	test('parsing stored profiles never throws', () => {
		for (const s of junk()) expect(() => parseProfile(s)).not.toThrow();
		for (const s of junk(100)) expect(() => parseProfile(JSON.stringify({ user: { username: s, role: s }, device: { id: s, name: s, mac: s } }))).not.toThrow();
	});

	test('storage that throws costs only the provisional sign-in', async () => {
		const flags = {
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
		const s = new SessionManager(scripted(['t1', new TypeError('Failed to fetch')]), flags);
		await s.restore();
		expect(s.status).toBe('signed-in');
		const s2 = new SessionManager(scripted([new TypeError('Failed to fetch')]), flags);
		await s2.restore();
		expect(s2.status).toBe('offline');
	});
});

describe('the last now-playing state', () => {
	const player = { playerid: '02:00:00:00:00:01', name: 'Lounge' };
	const now = { mode: 'play' as const, time: 98, playlist_cur_index: 2, playlist_tracks: 8, playlist_loop: [{ id: 5, title: 'Night Ferry' }] };

	test("is kept per user and per player, and given back only for that user's player", () => {
		saveLastKnown({ user: 'alice', player, now });
		expect(loadLastKnown('alice', player.playerid)).toEqual({ user: 'alice', player, now });
		expect(loadLastKnown('bob', player.playerid)).toBeNull();
		expect(loadLastKnown('alice', '02:00:00:00:00:02')).toBeNull();
		expect(loadLastKnown('', player.playerid)).toBeNull();
		expect(loadLastKnown('alice', '')).toBeNull();
		forgetLastKnown();
		expect(loadLastKnown('alice', player.playerid)).toBeNull();
	});

	test('anything malformed in storage is ignored', () => {
		const bad: unknown[] = [
			'not json',
			'[]',
			{ user: 'alice', player, now: [] },
			{ user: 'alice', player, now: { ...now, playlist_loop: 'x' } },
			{ user: 'alice', player, now: { ...now, playlist_loop: [1, 2] } },
			{ user: 'alice', player: { ...player, name: 3 }, now },
			{ user: 'alice', player: [player], now },
			{ user: 'alice', now },
		];
		for (const b of bad) {
			localStorage.setItem(LAST_KNOWN_KEY, typeof b === 'string' ? b : JSON.stringify(b));
			expect(loadLastKnown('alice', player.playerid)).toBeNull();
		}
		localStorage.setItem(LAST_KNOWN_KEY, JSON.stringify({ user: 'alice', player, now, pad: 'x'.repeat(LAST_KNOWN_MAX) }));
		expect(loadLastKnown('alice', player.playerid)).toBeNull();
		for (const s of junk()) {
			localStorage.setItem(LAST_KNOWN_KEY, s);
			expect(() => loadLastKnown('alice', player.playerid)).not.toThrow();
		}
	});

	test('too big to keep: forgotten rather than kept stale', () => {
		saveLastKnown({ user: 'alice', player, now });
		saveLastKnown({ user: 'alice', player, now: { ...now, playlist_loop: [{ id: 5, title: 'x'.repeat(LAST_KNOWN_MAX) }] } });
		expect(localStorage.getItem(LAST_KNOWN_KEY)).toBeNull();
	});
});

describe('the layout wires it up', () => {
	const layout = readFileSync(`${__dirname}/../src/routes/+layout.svelte`, 'utf8');

	test('address-bar routes open in the app', () => {
		expect(layout).toMatch(/\$effect\(\(\) => routeOutsideHashChanges\(\(\) => page\.url\.hash\)\)/);
	});

	test("this device's player waits for the sign-in to be confirmed", () => {
		expect(layout).toMatch(/app\.status === 'signed-in' && !app\.provisional\)/);
		expect(layout).toMatch(/downloads\.setUser\(user, !app\.provisional\)/);
	});

	test('the connection message (in the one status slot) has the id disabled controls point at', () => {
		const slot = readFileSync(`${__dirname}/../src/lib/components/StatusSlot.svelte`, 'utf8');
		expect(slot).toMatch(/role="status"\s+id=\{status\.kind === 'unreachable' \|\| status\.kind === 'busy' \? CONNECTION_ID : undefined\}/);
		expect(layout).toMatch(/<StatusSlot \/>/);
		expect(layout).toMatch(/app\.rememberNow\(\)/);
	});
});

describe('controls that need the server', () => {
	test('are disabled unless in reach, and point at the connection message when that is why', () => {
		expect(needsServer('online', '')).toEqual({ disabled: false });
		expect(needsServer('online', '', false)).toEqual({ disabled: true });
		expect(needsServer('unreachable', 'Can’t reach your music server.')).toEqual({ disabled: true, title: 'Can’t reach your music server.', 'aria-describedby': CONNECTION_ID });
		expect(needsServer('busy', 'Your music server is busy. Waiting for it…', true)).toMatchObject({ disabled: true, 'aria-describedby': CONNECTION_ID });
	});

	test('a control disabled for another reason does not blame the connection', () => {
		expect(explain(true, 'online', '')).toEqual({});
		expect(explain(false, 'unreachable', 'Can’t reach your music server.')).toEqual({});
		expect(explain(true, 'unreachable', '')).toEqual({});
	});
});
