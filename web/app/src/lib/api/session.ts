// The signed-in session: holds the access token in memory, renews it before it expires,
// and retries a request once with a fresh token when the gateway says it has expired.
import * as auth from './auth';
import { ApiError } from './http';
import type { Session } from './auth';

/** Renew this long before the 15-minute access token expires. */
const RENEW_BEFORE_MS = 5 * 60 * 1000;

/** `offline`: not signed in yet because the server can't be reached; retrying. */
export type SessionStatus = 'unknown' | 'offline' | 'signed-out' | 'signed-in';

/** Retry delays when the server can't be reached (then the last one, repeatedly). */
export const RETRY_MS = [2_000, 5_000, 15_000, 30_000, 60_000];

/**
 * Did the server refuse the sign-in? Only then is the user signed out. Anything else (no
 * network, a phone waking up, the gateway restarting behind the proxy) is retried, so a
 * moment offline never costs the user their sign-in.
 */
export const refused = (e: unknown) => e instanceof ApiError && e.status >= 400 && e.status < 500 && e.status !== 408 && e.status !== 429;
type Listener = (s: Session | null, status: SessionStatus) => void;

/** Set while a sign-out has not reached the server yet: the next page load must not restore. */
export const SIGNED_OUT_KEY = 'anywhere.signedOut';

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
	#listeners = new Set<Listener>();
	#timer: ReturnType<typeof setTimeout> | undefined;
	#renewing: Promise<boolean> | null = null;
	#retry: ReturnType<typeof setTimeout> | undefined;
	#attempt = 0;
	/**
	 * Bumped by every sign-out and sign-in: a refresh started before it must not apply its
	 * result (it would sign the user back in, or out of the new session).
	 */
	#gen = 0;

	constructor(
		private api: AuthApi = auth,
		private flags: Flags | undefined = safeLocal(),
	) {}

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

	#set(s: Session | null) {
		clearTimeout(this.#timer);
		clearTimeout(this.#retry);
		this.#attempt = 0;
		this.current = s;
		this.status = s ? 'signed-in' : 'signed-out';
		if (s) {
			const ms = Math.max(30_000, s.expires_in * 1000 - RENEW_BEFORE_MS);
			this.#timer = setTimeout(() => void this.renew(), ms);
		}
		this.#emit();
	}

	/** Try again later: the server couldn't be reached. */
	#later(what: () => void) {
		clearTimeout(this.#retry);
		const ms = RETRY_MS[Math.min(this.#attempt, RETRY_MS.length - 1)]!;
		this.#attempt++;
		this.#retry = setTimeout(what, ms);
	}

	/** Sign in silently with the refresh cookie, if this browser has one. */
	async restore(): Promise<boolean> {
		if (this.signOutPending) {
			// Signed out while offline: show the sign-in form, and finish signing out quietly.
			this.#set(null);
			void this.#finishSignOut();
			return false;
		}
		const gen = this.#gen;
		try {
			const s = await this.api.refresh();
			if (gen !== this.#gen) return false; // signed out (or in) meanwhile
			this.#set(s);
			return true;
		} catch (e) {
			if (gen !== this.#gen) return false;
			if (refused(e)) {
				this.#set(null);
			} else {
				// Can't tell yet: keep the sign-in and try again shortly.
				if (this.status !== 'offline') {
					this.status = 'offline';
					this.#emit();
				}
				this.#later(() => void this.restore());
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
		this.#set(null);
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
			const s = await this.api.refresh();
			if (gen !== this.#gen) return; // signed in meanwhile: leave the new session alone
			await this.api.logout(s.access_token);
			if (gen === this.#gen) this.#flag(false);
		} catch (e) {
			// Refused: the cookie is no longer valid, so there is nothing left to revoke.
			if (refused(e) && gen === this.#gen) this.#flag(false);
		}
	}

	/** Get a new access token. Concurrent callers share one refresh. */
	renew(): Promise<boolean> {
		if (!this.current) return Promise.resolve(false);
		if (this.#renewing) return this.#renewing;
		const gen = this.#gen;
		let p: Promise<boolean> | null = null;
		p = (async () => {
			await Promise.resolve(); // let `#renewing` be set before anything below runs
			try {
				const s = await this.api.refresh();
				if (gen !== this.#gen) return false; // signed out meanwhile: stay signed out
				this.#set(s);
				return true;
			} catch (e) {
				if (gen !== this.#gen) return false;
				if (refused(e)) this.#set(null);
				else this.#later(() => void this.renew()); // stay signed in; try again soon
				return false;
			} finally {
				if (this.#renewing === p) this.#renewing = null;
			}
		})();
		this.#renewing = p;
		return p;
	}

	/** Run an authenticated call; on 401, renew the token once and retry. */
	async authed<T>(fn: (token: string) => Promise<T>): Promise<T> {
		if (!this.current) throw new ApiError(401, 'not signed in');
		const token = this.token;
		try {
			return await fn(token);
		} catch (e) {
			if (!(e instanceof ApiError) || e.status !== 401) throw e;
			// Another call may have renewed it already.
			if (this.token === token && !(await this.renew())) throw e;
			if (!this.current) throw e;
			return fn(this.token);
		}
	}
}
