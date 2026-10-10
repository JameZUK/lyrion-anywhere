// The signed-in session: holds the access token in memory, renews it before it expires,
// and retries a request once with a fresh token when the gateway says it has expired.
import * as auth from './auth';
import { ApiError } from './http';
import type { Session } from './auth';

/** Renew this long before the 15-minute access token expires. */
const RENEW_BEFORE_MS = 5 * 60 * 1000;

export type SessionStatus = 'unknown' | 'signed-out' | 'signed-in';
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
		this.current = s;
		this.status = s ? 'signed-in' : 'signed-out';
		if (s) {
			const ms = Math.max(30_000, s.expires_in * 1000 - RENEW_BEFORE_MS);
			this.#timer = setTimeout(() => void this.renew(), ms);
		}
		this.#emit();
	}

	/** Sign in silently with the refresh cookie, if this browser has one. */
	async restore(): Promise<boolean> {
		try {
			this.#set(await this.api.refresh());
			return true;
		} catch {
			this.#set(null);
			return false;
		}
	}

	async signIn(username: string, password: string, totp?: string): Promise<void> {
		this.#set(await this.api.login(username, password, totp));
	}

	async signOut(): Promise<void> {
		const token = this.token;
		this.#set(null);
		if (token) await this.api.logout(token).catch(() => {});
	}

	/** Get a new access token. Concurrent callers share one refresh. */
	renew(): Promise<boolean> {
		this.#renewing ??= (async () => {
			try {
				this.#set(await this.api.refresh());
				return true;
			} catch {
				this.#set(null);
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
