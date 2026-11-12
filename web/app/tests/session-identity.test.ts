// Review M8: a renewal that comes back as another user or device, and sign-outs between tabs.
// Tab A is alice; in tab B she signs out and bob signs in; tab A's next renewal returns bob's
// session (the cookie is shared). That is a sign-out followed by a sign-in, never a quiet
// renewal: nothing of alice's is retried as bob.
import { readFileSync } from 'node:fs';
import { SessionManager, type AuthApi } from '#lib/api/session';
import { BUS_NAME, SIGNAL_KEY, browserBus, parseSignal, type SessionBus, type SessionSignal } from '#lib/api/session-bus';
import { ApiError } from '#lib/api/http';
import type { Session } from '#lib/api/auth';
import { switchedUser } from '#lib/state/per-user';

const as = (username: string, token: string, device = 3): Session => ({
	access_token: token,
	expires_in: 900,
	user: { username, role: 'listener' },
	device: { id: device, name: 'Firefox on Linux' },
});
const refusal = () => new ApiError(401, 'invalid credentials', false, undefined, { error: 'invalid credentials' });

/** A refresh that answers each call from the list in turn. */
function scripted(answers: (Session | Error)[]): AuthApi & { refresh: ReturnType<typeof vi.fn>; logout: ReturnType<typeof vi.fn> } {
	return {
		login: vi.fn(async () => as('alice', 't-login')),
		refresh: vi.fn(async () => {
			const a = answers.shift();
			if (!a) throw refusal();
			if (a instanceof Error) throw a;
			return a;
		}),
		logout: vi.fn(async () => {}),
	};
}

const expired = () => new ApiError(401, 'invalid credentials', false, undefined, { error: 'invalid credentials' });

afterEach(() => {
	vi.useRealTimers();
	localStorage.clear();
});

describe('a renewal that returns someone else', () => {
	test('is a sign-out then a sign-in, and the pending call is not retried as them', async () => {
		const s = new SessionManager(scripted([as('alice', 'a1'), as('bob', 'b1')]), undefined);
		await s.restore();
		const seen: { st: string; user: string | null }[] = [];
		s.subscribe((cur, st) => seen.push({ st, user: cur?.user.username ?? null }));
		const fn = vi.fn(async (t: string) => {
			if (t === 'a1') throw expired();
			return `as ${t}`;
		});
		await expect(s.authed(fn)).rejects.toMatchObject({ status: 401 });
		expect(fn).toHaveBeenCalledTimes(1);
		expect(fn).toHaveBeenCalledWith('a1');
		// Alice's sign-in ended (everything of hers is reset), then bob's started.
		expect(seen.slice(1)).toEqual([
			{ st: 'signed-out', user: null },
			{ st: 'signed-in', user: 'bob' },
		]);
		expect(s.status).toBe('signed-in');
		expect(s.current?.user.username).toBe('bob');
		expect(s.token).toBe('b1');
	});

	test('another device of the same user counts as someone else too', async () => {
		const s = new SessionManager(scripted([as('alice', 'a1', 3), as('alice', 'a2', 9)]), undefined);
		await s.restore();
		const states: string[] = [];
		s.subscribe((_, st) => states.push(st));
		expect(await s.renew()).toBe(false);
		expect(states.slice(1)).toEqual(['signed-out', 'signed-in']);
		expect(s.current?.device.id).toBe(9);
	});

	test('the same user on the same device is an ordinary renewal (a new role shows, the call is retried)', async () => {
		const s = new SessionManager(scripted([as('alice', 'a1'), { ...as('alice', 'a2'), user: { username: 'alice', role: 'admin' } }]), undefined);
		await s.restore();
		const states: string[] = [];
		s.subscribe((_, st) => states.push(st));
		const r = await s.authed(async (t) => {
			if (t === 'a1') throw expired();
			return t;
		});
		expect(r).toBe('a2');
		expect(states).not.toContain('signed-out');
		expect(s.current?.user.role).toBe('admin');
	});

	test('calls waiting on the same renewal are none of them retried as the new user', async () => {
		const s = new SessionManager(scripted([as('alice', 'a1'), as('bob', 'b1')]), undefined);
		await s.restore();
		const tokens: string[] = [];
		const call = async (t: string) => {
			tokens.push(t);
			if (t === 'a1') throw expired();
			return t;
		};
		const results = await Promise.allSettled([s.authed(call), s.authed(call), s.authed(call)]);
		expect(results.every((r) => r.status === 'rejected')).toBe(true);
		expect(tokens).toEqual(['a1', 'a1', 'a1']);
	});

	test('a call started before the switch and failing after it is not retried either', async () => {
		const auth = scripted([as('alice', 'a1'), as('bob', 'b1')]);
		const s = new SessionManager(auth, undefined);
		await s.restore();
		let fail!: (e: unknown) => void;
		const slow = vi.fn((t: string) => (t === 'a1' ? new Promise<string>((_, rej) => (fail = rej)) : Promise.resolve(t)));
		const pending = s.authed(slow);
		// Meanwhile another call's renewal switches to bob.
		await s.authed(async (t) => (t === 'a1' ? Promise.reject(expired()) : t)).catch(() => {});
		expect(s.current?.user.username).toBe('bob');
		fail(expired());
		await expect(pending).rejects.toMatchObject({ status: 401 });
		expect(slow).toHaveBeenCalledTimes(1);
		expect(auth.refresh).toHaveBeenCalledTimes(2);
	});

	test('a page that came back provisionally as alice and is confirmed as bob starts afresh as bob', async () => {
		vi.useFakeTimers();
		const flags = new Map<string, string>();
		const store = { getItem: (k: string) => flags.get(k) ?? null, setItem: (k: string, v: string) => void flags.set(k, v), removeItem: (k: string) => void flags.delete(k) };
		// An earlier page load, signed in as alice.
		await new SessionManager(scripted([as('alice', 'a0')]), store).restore();
		const s = new SessionManager(scripted([new TypeError('Failed to fetch'), as('bob', 'b1')]), store);
		const seen: { st: string; user: string | null }[] = [];
		s.subscribe((cur, st) => seen.push({ st, user: cur?.user.username ?? null }));
		await s.restore();
		expect(s.provisional).toBe(true);
		expect(s.current?.user.username).toBe('alice');
		await vi.advanceTimersByTimeAsync(5_000);
		expect(seen.slice(1)).toEqual([
			{ st: 'signed-in', user: 'alice' },
			{ st: 'signed-out', user: null },
			{ st: 'signed-in', user: 'bob' },
		]);
		expect(s.provisional).toBe(false);
		expect(flags.get('anywhere.session')).toContain('bob');
	});

	test('a call made while provisional as alice never runs once the server says bob', async () => {
		const flags = new Map<string, string>();
		const store = { getItem: (k: string) => flags.get(k) ?? null, setItem: (k: string, v: string) => void flags.set(k, v), removeItem: (k: string) => void flags.delete(k) };
		await new SessionManager(scripted([as('alice', 'a0')]), store).restore();
		const s = new SessionManager(scripted([new TypeError('Failed to fetch'), as('bob', 'b1')]), store);
		await s.restore();
		expect(s.provisional).toBe(true);
		const fn = vi.fn(async (t: string) => t);
		await expect(s.authed(fn)).rejects.toMatchObject({ status: 401 });
		expect(fn).not.toHaveBeenCalled();
		expect(s.current?.user.username).toBe('bob');
	});
});

/** A bus joining the managers of several "tabs" in this test, delivering as browsers do (later, never to the sender). */
function fakeBuses() {
	const all = new Set<{ fns: Set<(m: SessionSignal) => void> }>();
	const make = (): SessionBus & { sent: SessionSignal[] } => {
		const me = { fns: new Set<(m: SessionSignal) => void>() };
		all.add(me);
		const sent: SessionSignal[] = [];
		return {
			sent,
			post(m) {
				sent.push(m);
				for (const other of all) if (other !== me) for (const fn of other.fns) setTimeout(() => fn(m), 0);
			},
			listen(fn) {
				me.fns.add(fn);
				return () => me.fns.delete(fn);
			},
		};
	};
	return { make };
}

describe('signing out in one tab', () => {
	const settle = () => new Promise((r) => setTimeout(r, 5));

	test('signs the other tabs out at once, without telling the server again', async () => {
		const buses = fakeBuses();
		const authA = scripted([as('alice', 'a1')]);
		const authB = scripted([as('alice', 'a2')]);
		const a = new SessionManager(authA, undefined, buses.make());
		const busB = buses.make();
		const b = new SessionManager(authB, undefined, busB);
		await a.restore();
		await b.restore();
		await b.signOut();
		expect(busB.sent).toEqual([{ type: 'signed-out' }]);
		await settle();
		expect(a.status).toBe('signed-out');
		expect(a.token).toBe('');
		expect(a.endedOnPurpose).toBe(true);
		expect(authA.logout).not.toHaveBeenCalled();
		expect(authB.logout).toHaveBeenCalledTimes(1);
	});

	test('a renewal in flight in the other tab does not sign it back in', async () => {
		const buses = fakeBuses();
		let late!: (s: Session) => void;
		const authA = scripted([as('alice', 'a1')]);
		const a = new SessionManager(authA, undefined, buses.make());
		const b = new SessionManager(scripted([as('alice', 'a2')]), undefined, buses.make());
		await a.restore();
		await b.restore();
		authA.refresh.mockImplementation(() => new Promise<Session>((r) => (late = r)));
		const renewing = a.renew();
		await Promise.resolve();
		await Promise.resolve();
		await b.signOut();
		await settle();
		late(as('alice', 'a-late'));
		expect(await renewing).toBe(false);
		expect(a.status).toBe('signed-out');
	});

	test('retries pending in the other tab stop; a tab already signed out ignores it', async () => {
		vi.useFakeTimers();
		const buses = fakeBuses();
		const authA = scripted([new TypeError('Failed to fetch')]);
		const a = new SessionManager(authA, undefined, buses.make());
		const b = new SessionManager(scripted([as('alice', 'a2')]), undefined, buses.make());
		await a.restore(); // offline, retrying
		await b.restore();
		await b.signOut();
		await vi.advanceTimersByTimeAsync(10);
		expect(a.status).toBe('signed-out');
		await vi.advanceTimersByTimeAsync(120_000);
		expect(authA.refresh).toHaveBeenCalledTimes(1);
		const states: string[] = [];
		a.subscribe((_, st) => states.push(st));
		await b.signOut();
		await vi.advanceTimersByTimeAsync(10);
		expect(states).toEqual(['signed-out']); // nothing more
	});

	test('a sign-out the server ended (a refusal) is not on purpose; one asked for is', async () => {
		const s = new SessionManager(scripted([as('alice', 'a1')]), undefined);
		await s.restore();
		await s.authed(async () => Promise.reject(expired())).catch(() => {});
		expect(s.status).toBe('signed-out');
		expect(s.endedOnPurpose).toBe(false);
		const t = new SessionManager(scripted([as('alice', 'a1')]), undefined);
		await t.restore();
		await t.signOut();
		expect(t.endedOnPurpose).toBe(true);
		await t.signIn('alice', 'pw');
		expect(t.endedOnPurpose).toBe(false);
	});
});

describe('the bus between tabs', () => {
	test('only the one known message is accepted', () => {
		expect(parseSignal({ type: 'signed-out' })).toEqual({ type: 'signed-out' });
		expect(parseSignal({ type: 'signed-out', user: 'mallory' })).toEqual({ type: 'signed-out' }); // extras dropped
		for (const bad of [null, undefined, 'signed-out', 1, [], [{ type: 'signed-out' }], {}, { type: 'signed-in' }, { type: 'SIGNED-OUT' }, { type: ['signed-out'] }]) {
			expect(parseSignal(bad)).toBeNull();
		}
	});

	test('a BroadcastChannel where there is one: posts reach the other channels, junk is ignored', () => {
		const channels: FakeChannel[] = [];
		class FakeChannel {
			fns = new Set<(e: { data: unknown }) => void>();
			constructor(public name: string) {
				channels.push(this);
			}
			postMessage(m: unknown) {
				for (const c of channels) if (c !== this && c.name === this.name) for (const fn of c.fns) fn({ data: structuredClone(m) });
			}
			addEventListener(_t: 'message', fn: (e: { data: unknown }) => void) {
				this.fns.add(fn);
			}
			removeEventListener(_t: 'message', fn: (e: { data: unknown }) => void) {
				this.fns.delete(fn);
			}
		}
		const a = browserBus({ BroadcastChannel: FakeChannel })!;
		const b = browserBus({ BroadcastChannel: FakeChannel })!;
		expect(channels.map((c) => c.name)).toEqual([BUS_NAME, BUS_NAME]);
		const heard: SessionSignal[] = [];
		const stop = b.listen((m) => heard.push(m));
		a.post({ type: 'signed-out' });
		channels[0]!.postMessage({ type: 'evil' });
		channels[0]!.postMessage('signed-out');
		expect(heard).toEqual([{ type: 'signed-out' }]);
		stop();
		a.post({ type: 'signed-out' });
		expect(heard).toHaveLength(1);
	});

	test('without one, a storage event: written and removed at once, checked when heard', () => {
		const store = new Map<string, string>();
		const writes: string[] = [];
		const storage = {
			setItem: (k: string, v: string) => (writes.push(`set ${k}`), void store.set(k, v)),
			removeItem: (k: string) => (writes.push(`remove ${k}`), void store.delete(k)),
		};
		const events = new EventTarget();
		const bus = browserBus({ BroadcastChannel: undefined, storage, events })!;
		const heard: SessionSignal[] = [];
		bus.listen((m) => heard.push(m));
		bus.post({ type: 'signed-out' });
		expect(writes).toEqual([`set ${SIGNAL_KEY}`, `remove ${SIGNAL_KEY}`]);
		expect(store.has(SIGNAL_KEY)).toBe(false);
		// What another tab's write looks like here.
		const storageEvent = (key: string | null, newValue: string | null) => Object.assign(new Event('storage'), { key, newValue });
		events.dispatchEvent(storageEvent(SIGNAL_KEY, JSON.stringify({ type: 'signed-out', at: 1, n: 0.5 })));
		events.dispatchEvent(storageEvent(SIGNAL_KEY, null)); // the removal
		events.dispatchEvent(storageEvent('anywhere.other', JSON.stringify({ type: 'signed-out' })));
		events.dispatchEvent(storageEvent(SIGNAL_KEY, '{not json'));
		events.dispatchEvent(storageEvent(SIGNAL_KEY, JSON.stringify({ type: 'signed-in' })));
		events.dispatchEvent(storageEvent(SIGNAL_KEY, JSON.stringify({ type: 'signed-out', pad: 'x'.repeat(600) })));
		expect(heard).toEqual([{ type: 'signed-out' }]);
	});

	test('nothing to send with: no bus, and the session works without one', async () => {
		expect(browserBus({ BroadcastChannel: undefined, storage: undefined, events: new EventTarget() })).toBeUndefined();
		const throwing = class {
			constructor() {
				throw new DOMException('denied', 'SecurityError');
			}
		} as unknown as NonNullable<Parameters<typeof browserBus>[0]>['BroadcastChannel'];
		// A channel the browser refuses falls back to storage.
		expect(browserBus({ BroadcastChannel: throwing, storage: { setItem() {}, removeItem() {} }, events: new EventTarget() })).toBeDefined();
		const s = new SessionManager(scripted([as('alice', 'a1')]), undefined, undefined);
		await s.restore();
		await s.signOut();
		expect(s.status).toBe('signed-out');
	});

	test('storage that throws never breaks a sign-out', async () => {
		const bus = browserBus({
			BroadcastChannel: undefined,
			storage: {
				setItem() {
					throw new DOMException('full', 'QuotaExceededError');
				},
				removeItem() {},
			},
			events: new EventTarget(),
		})!;
		expect(() => bus.post({ type: 'signed-out' })).not.toThrow();
	});
});

describe('the app follows a change of user', () => {
	test('signed in as someone other than the user last seen, with no sign-out seen between', () => {
		expect(switchedUser('alice', 'signed-in', 'bob')).toBe(true);
		expect(switchedUser('alice', 'signed-in', 'alice')).toBe(false);
		expect(switchedUser(null, 'signed-in', 'bob')).toBe(false); // the first sign-in
		expect(switchedUser('alice', 'signed-out', null)).toBe(false);
		expect(switchedUser('alice', 'offline', null)).toBe(false);
		expect(switchedUser('alice', 'signed-in', null)).toBe(false);
	});

	// The layout's effect may see only the end of a sign-out followed at once by a sign-in
	// (Svelte runs it once for both): it ends the old user's offline player, library and
	// downloads itself before starting the new user's.
	test('the layout ends the old user’s things before starting the new user’s', () => {
		const layout = readFileSync(`${__dirname}/../src/routes/+layout.svelte`, 'utf8');
		const effect = layout.slice(layout.indexOf('let lastStatus'), layout.indexOf('// The browser lost its connection'));
		const switched = effect.indexOf('switchedUser(');
		const start = effect.indexOf("if (app.status === 'signed-in' && user)");
		expect(switched).toBeGreaterThan(-1);
		expect(switched).toBeLessThan(start);
		const ending = effect.slice(switched, start);
		for (const step of ['handoff.reset()', 'offlinePlayer.stop()', 'library.stop()', 'forgetOwner()', 'downloads.setUser(null)', 'plays.setUser(null)']) expect(ending, step).toContain(step);
		// A sign-out asked for (here or in another tab) forgets the offline queue.
		expect(effect).toContain('session.endedOnPurpose');
	});

	test('the app’s session listens to the other tabs', () => {
		const src = readFileSync(`${__dirname}/../src/lib/state/app.svelte.ts`, 'utf8');
		expect(src).toMatch(/new SessionManager\([^)]*browserBus\(\)\)/);
	});
});
