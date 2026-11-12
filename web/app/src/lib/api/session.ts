// The signed-in session: holds the access token in memory, renews it before it expires,
// and retries a request once with a fresh token when the gateway says it has expired.
import * as auth from './auth';
import { ApiError } from './http';
import type { RefreshReason, Session } from './auth';
import type { SessionBus, SessionSignal } from './session-bus';

/** Renew this long before the 15-minute access token expires. */
const RENEW_BEFORE_MS = 5 * 60 * 1000;

/** `offline`: not signed in yet because the server can't be reached; retrying. */
export type SessionStatus = 'unknown' | 'offline' | 'signed-out' | 'signed-in';

/** Retry delays when the server can't be reached (then the last one, repeatedly). */
export const RETRY_MS = [2_000, 5_000, 15_000, 30_000, 60_000];

/**
 * Did the gateway itself refuse the sign-in? That is a 401 carrying its JSON `error`
 * (docs/PROTOCOL.md), and only then is the user signed out. Anything else is retried: no
 * network, a phone waking up, the gateway restarting behind the proxy, and a proxy's, CDN's
 * or firewall's own page of any status (a challenge page, a 404 from a misrouted path). A
 * moment like that never costs the user their sign-in, their profile or their downloads.
 */
export const refused = (e: unknown) => e instanceof ApiError && e.status === 401 && typeof e.body.error === 'string';
type Listener = (s: Session | null, status: SessionStatus) => void;

/** Set while a sign-out has not reached the server yet: the next page load must not restore. */
export const SIGNED_OUT_KEY = 'anywhere.signedOut';

/**
 * Who is signed in on this device (never a token: the access token stays in memory and the
 * refresh token in its HttpOnly cookie). A page loaded while the server is out of reach uses
 * it to bring the app back as it was, rather than a bare "can't reach" page; the server still
 * decides once it answers.
 */
export const PROFILE_KEY = 'anywhere.session';

/** The part of a session kept across page loads. */
export type Profile = Pick<Session, 'user' | 'device'>;

/**
 * The same sign-in: the same user on the same device. A refresh that answers with anyone
 * else means the shared cookie now belongs to another sign-in (signed out and in again in
 * another tab), which is never taken for a renewal of this one.
 */
export const sameSignIn = (a: Profile, b: Profile): boolean => a.user.username === b.user.username && a.device.id === b.device.id;

const text = (v: unknown, max: number): v is string => typeof v === 'string' && v.length > 0 && v.length <= max;

/** A stored profile, checked field by field: anything malformed is no profile at all. */
export function parseProfile(raw: string | null | undefined): Profile | null {
	if (!raw || raw.length > 2048) return null;
	let v: unknown;
	try {
		v = JSON.parse(raw);
	} catch {
		return null;
	}
	if (!v || typeof v !== 'object') return null;
	const { user, device } = v as Record<string, unknown>;
	if (!user || typeof user !== 'object' || !device || typeof device !== 'object') return null;
	const u = user as Record<string, unknown>;
	const d = device as Record<string, unknown>;
	if (!text(u.username, 200) || (u.role !== 'admin' && u.role !== 'listener')) return null;
	if (!Number.isSafeInteger(d.id) || (d.id as number) <= 0 || typeof d.name !== 'string' || d.name.length > 200) return null;
	// The player id only if it looks like one (the app checks it again where it uses it).
	const mac = typeof d.mac === 'string' && /^[0-9a-f]{2}(:[0-9a-f]{2}){5}$/i.test(d.mac) ? d.mac : undefined;
	return {
		user: { username: u.username, role: u.role },
		device: { id: d.id as number, name: d.name, ...(mac ? { mac } : {}) },
	};
}

type Flags = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

function safeLocal(): Flags | undefined {
	try {
		return globalThis.localStorage;
	} catch {
		return undefined;
	}
}

export interface AuthApi {
	login: typeof auth.login;
	refresh: typeof auth.refresh;
	logout: typeof auth.logout;
}

export class SessionManager {
	current: Session | null = null;
	status: SessionStatus = 'unknown';
	/**
	 * Signed in as far as this device knows, but not confirmed yet: the page loaded while the
	 * server was out of reach. There is no token (requests wait for a real one); the sign-in is
	 * confirmed, or ended if the server refuses it, as soon as the server answers.
	 */
	provisional = false;
	#listeners = new Set<Listener>();
	#timer: ReturnType<typeof setTimeout> | undefined;
	#renewing: Promise<boolean> | null = null;
	#retry: ReturnType<typeof setTimeout> | undefined;
	#attempt = 0;
	/**
	 * Bumped by every sign-out and sign-in (and a switch to another sign-in): a refresh
	 * started before it must not apply its result (it would sign the user back in, or out of
	 * the new session), and a call made before it is never retried after it.
	 */
	#gen = 0;
	/** The last sign-out was asked for (here or in another tab), not the server ending it. */
	#onPurpose = false;

	/**
	 * `bus`: the other tabs of this browser (see session-bus.ts). A sign-out here is told to
	 * them, and one there ends the sign-in here.
	 */
	constructor(
		private api: AuthApi = auth,
		private flags: Flags | undefined = safeLocal(),
		private bus: SessionBus | undefined = undefined,
	) {
		bus?.listen((m) => this.#heard(m));
	}

	/** Signed out because someone asked to be (here or in another tab), not by the server. */
	get endedOnPurpose(): boolean {
		return this.status === 'signed-out' && (this.#onPurpose || this.signOutPending);
	}

	/** A sign-out this browser has not managed to tell the server about yet. */
	get signOutPending(): boolean {
		try {
			return this.flags?.getItem(SIGNED_OUT_KEY) === '1';
		} catch {
			return false;
		}
	}

	#flag(on: boolean) {
		try {
			if (on) this.flags?.setItem(SIGNED_OUT_KEY, '1');
			else this.flags?.removeItem(SIGNED_OUT_KEY);
		} catch {
			/* storage blocked: best effort */
		}
	}

	get token(): string {
		return this.current?.access_token ?? '';
	}

	subscribe(fn: Listener): () => void {
		this.#listeners.add(fn);
		fn(this.current, this.status);
		return () => this.#listeners.delete(fn);
	}

	#emit() {
		for (const fn of this.#listeners) fn(this.current, this.status);
	}

	#profile(s: Session | null) {
		try {
			if (s) this.flags?.setItem(PROFILE_KEY, JSON.stringify({ user: s.user, device: s.device }));
			else this.flags?.removeItem(PROFILE_KEY);
		} catch {
			/* storage blocked: a load while out of reach shows offline mode instead */
		}
	}

	#storedProfile(): Profile | null {
		try {
			return parseProfile(this.flags?.getItem(PROFILE_KEY));
		} catch {
			return null;
		}
	}

	#set(s: Session | null) {
		clearTimeout(this.#timer);
		clearTimeout(this.#retry);
		this.#attempt = 0;
		if (s) this.#onPurpose = false;
		this.current = s;
		this.provisional = false;
		this.status = s ? 'signed-in' : 'signed-out';
		this.#profile(s);
		if (s) {
			const ms = Math.max(30_000, s.expires_in * 1000 - RENEW_BEFORE_MS);
			this.#timer = setTimeout(() => void this.renew('timer'), ms);
		}
		this.#emit();
	}

	/**
	 * Take a session the server gave back for this sign-in. If it is someone else's (another
	 * user or device: see sameSignIn), this sign-in has ended: everything of it stops and is
	 * reset as at a sign-out, then the new one starts afresh, and nothing pending is retried
	 * under it. Returns whether it was this same sign-in.
	 */
	#apply(s: Session): boolean {
		if (!this.current || sameSignIn(this.current, s)) {
			this.#set(s);
			return true;
		}
		this.#gen++;
		this.#renewing = null;
		this.#set(null);
		this.#set(s);
		return false;
	}

	/** A message from another tab of this browser. */
	#heard(m: SessionSignal) {
		if (m.type !== 'signed-out' || this.status === 'signed-out') return;
		// Signed out there: the shared refresh cookie is gone, so this sign-in is too. That tab
		// has told the server; here it only ends.
		this.#gen++;
		this.#renewing = null;
		this.#onPurpose = true;
		this.#set(null);
	}

	/** Try again later: the server couldn't be reached. */
	#later(what: () => void) {
		clearTimeout(this.#retry);
		const ms = RETRY_MS[Math.min(this.#attempt, RETRY_MS.length - 1)]!;
		this.#attempt++;
		this.#retry = setTimeout(what, ms);
	}

	/** Sign in silently with the refresh cookie, if this browser has one. */
	async restore(reason: RefreshReason = 'load'): Promise<boolean> {
		if (this.signOutPending) {
			// Signed out while offline: show the sign-in form, and finish signing out quietly.
			this.#set(null);
			void this.#finishSignOut();
			return false;
		}
		const gen = this.#gen;
		try {
			const s = await this.api.refresh(reason);
			if (gen !== this.#gen) return false; // signed out (or in) meanwhile
			// A page that came back provisionally as someone else starts afresh as this one.
			this.#apply(s);
			return true;
		} catch (e) {
			if (gen !== this.#gen) return false;
			if (refused(e)) {
				this.#set(null);
			} else {
				// Can't tell yet: keep the sign-in and try again shortly. Someone was signed in
				// here: carry on as them meanwhile (provisionally, with no token), so a page
				// loaded while the server is away brings the app back, not a bare "can't reach"
				// page. Offline mode is left for a device with nobody signed in.
				const profile = this.status === 'signed-in' ? null : this.#storedProfile();
				if (profile) {
					this.current = { access_token: '', expires_in: 0, ...profile };
					this.provisional = true;
					this.status = 'signed-in';
					this.#emit();
				} else if (this.status !== 'offline' && this.status !== 'signed-in') {
					this.status = 'offline';
					this.#emit();
				}
				this.#later(() => void this.restore('retry'));
			}
			return false;
		}
	}

	async signIn(username: string, password: string, totp?: string): Promise<void> {
		const s = await this.api.login(username, password, totp);
		this.#gen++;
		this.#renewing = null;
		this.#flag(false);
		this.#set(s);
	}

	/**
	 * Sign out. The flag is set first and cleared only once the server has revoked the
	 * session, so a sign-out that can't reach the server still holds after a reload.
	 */
	async signOut(): Promise<void> {
		const token = this.token;
		this.#gen++;
		this.#renewing = null;
		this.#flag(true);
		this.#onPurpose = true;
		this.#set(null);
		// The other tabs share the cookie: they are signed out with this one.
		this.bus?.post({ type: 'signed-out' });
		if (!token) return; // finished on the next page load (see restore)
		const gen = this.#gen;
		try {
			await this.api.logout(token);
			if (gen === this.#gen) this.#flag(false);
		} catch {
			/* offline: the flag stays, and the next page load finishes it */
		}
	}

	/** Revoke the session the refresh cookie still holds, then forget the pending sign-out. */
	async #finishSignOut(): Promise<void> {
		const gen = this.#gen;
		try {
			const s = await this.api.refresh('signout');
			if (gen !== this.#gen) return; // signed in meanwhile: leave the new session alone
			await this.api.logout(s.access_token);
			if (gen === this.#gen) this.#flag(false);
		} catch (e) {
			// Refused: the cookie is no longer valid, so there is nothing left to revoke.
			if (refused(e) && gen === this.#gen) this.#flag(false);
		}
	}

	/** Get a new access token. Concurrent callers share one refresh. */
	renew(reason: RefreshReason = '401'): Promise<boolean> {
		if (!this.current) return Promise.resolve(false);
		if (this.#renewing) return this.#renewing;
		const gen = this.#gen;
		let p: Promise<boolean> | null = null;
		p = (async () => {
			await Promise.resolve(); // let `#renewing` be set before anything below runs
			try {
				const s = await this.api.refresh(reason);
				if (gen !== this.#gen) return false; // signed out meanwhile: stay signed out
				// Someone else's session: not a renewal of this one, so nothing is retried.
				return this.#apply(s);
			} catch (e) {
				if (gen !== this.#gen) return false;
				if (refused(e)) this.#set(null);
				else this.#later(() => void this.renew('retry')); // stay signed in; try again soon
				return false;
			} finally {
				if (this.#renewing === p) this.#renewing = null;
			}
		})();
		this.#renewing = p;
		return p;
	}

	/** This device was renamed: show the new name everywhere at once. */
	setDeviceName(name: string) {
		if (!this.current) return;
		this.current = { ...this.current, device: { ...this.current.device, name } };
		this.#emit();
	}

	/**
	 * Before a call: signed in, and with a real token (a sign-in not confirmed yet gets one
	 * first, so no request ever goes with an empty one). Returns the sign-in's generation.
	 */
	async #ready(): Promise<number> {
		if (!this.current) throw new ApiError(401, 'not signed in');
		const gen = this.#gen;
		if (this.provisional) {
			const ok = await this.renew();
			// Signed out, or confirmed as someone else: the call was made for the old sign-in.
			if (!this.current || gen !== this.#gen) throw new ApiError(401, 'not signed in');
			if (!ok) throw new TypeError('Can’t reach your music server.');
		}
		return gen;
	}

	/** Run an authenticated call; on 401, renew the token once and retry. */
	async authed<T>(fn: (token: string) => Promise<T>): Promise<T> {
		const gen = await this.#ready();
		const token = this.token;
		try {
			return await fn(token);
		} catch (e) {
			if (!(e instanceof ApiError) || e.status !== 401) throw e;
			// Another call may have renewed it already.
			if (this.token === token && !(await this.renew())) throw e;
			// Signed out, or in as someone else, meanwhile: never retried under another sign-in.
			if (!this.current || gen !== this.#gen) throw e;
			return fn(this.token);
		}
	}

	/**
	 * Run a call that carries a password or a two-step code. The gateway answers wrong
	 * credentials with 403, never 401 (docs/PROTOCOL.md), so a 401 is about the token: it is
	 * renewed for the next try, but the call is not sent again by itself. What the person
	 * typed is sent once per tap, and fails with {@link RenewedFirst} instead.
	 */
	async authedOnce<T>(fn: (token: string) => Promise<T>): Promise<T> {
		const gen = await this.#ready();
		const token = this.token;
		try {
			return await fn(token);
		} catch (e) {
			if (!(e instanceof ApiError) || e.status !== 401) throw e;
			const ok = this.token === token ? await this.renew() : true;
			if (!this.current || gen !== this.#gen) throw e;
			if (!ok) throw new TypeError('Can’t reach your music server.');
			throw new RenewedFirst();
		}
	}
}

/**
 * A call carrying a password or code met an expired token. The token has been renewed, and
 * nothing was checked or changed: the person tries again (see SessionManager.authedOnce).
 */
export class RenewedFirst extends Error {
	constructor() {
		super('Your sign-in had to be renewed first, so nothing was changed. Try again.');
		this.name = 'RenewedFirst';
	}
}
