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

	constructor(private api: AuthApi = auth) {}

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
		try {
			this.#set(await this.api.refresh());
			return true;
		} catch (e) {
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
		this.#set(await this.api.login(username, password, totp));
	}

	async signOut(): Promise<void> {
		const token = this.token;
		this.#set(null);
		clearTimeout(this.#retry);
		if (token) await this.api.logout(token).catch(() => {});
	}

	/** Get a new access token. Concurrent callers share one refresh. */
	renew(): Promise<boolean> {
		this.#renewing ??= (async () => {
			try {
				this.#set(await this.api.refresh());
				return true;
			} catch (e) {
				if (refused(e)) this.#set(null);
				else this.#later(() => void this.renew()); // stay signed in; try again soon
				return false;
			} finally {
				this.#renewing = null;
			}
		})();
		return this.#renewing;
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
