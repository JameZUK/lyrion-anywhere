// LMS commands through the gateway's allow-list (POST /v1/rpc) and artwork (GET /v1/art).
import { ApiError, bearer, callJson, isBusy } from './http';
import type { SessionManager } from './session';
import { isNetworkError } from '$lib/state/reachability';

export type Command = (string | number)[];

/** Runs at most `limit` tasks at once; the rest wait in order. */
export class Limiter {
	#active = 0;
	#queue: (() => void)[] = [];
	constructor(readonly limit: number) {}

	async run<T>(task: () => Promise<T>): Promise<T> {
		if (this.#active >= this.limit) await new Promise<void>((r) => this.#queue.push(r));
		this.#active++;
		try {
			return await task();
		} finally {
			this.#active--;
			this.#queue.shift()?.();
		}
	}
}

const ART_PREFIX = '/v1/art/';

/**
 * The same-origin URL (path only) for an artwork path, or null if the path would resolve
 * anywhere outside /v1/art/ (dot segments, encoded dots, backslashes, a query or fragment).
 * The browser's own URL parser decides, so whatever it would fetch is what is checked.
 */
export function artUrl(path: string): string | null {
	if (typeof path !== 'string' || !path || path.length > 4096 || /[?#\\\s]|%5c|\/\/|^\//i.test(path)) return null;
	const base = globalThis.location?.origin && globalThis.location.origin !== 'null' ? globalThis.location.origin : 'http://localhost';
	let u: URL;
	try {
		u = new URL(ART_PREFIX + path, base);
	} catch {
		return null;
	}
	if (u.origin !== new URL(base).origin || u.search || u.hash) return null;
	if (!u.pathname.startsWith(ART_PREFIX) || u.pathname.length === ART_PREFIX.length) return null;
	// Normalisation must not have changed the path (e.g. resolved "a/../b" or ".%2E").
	if (u.pathname !== ART_PREFIX + path) return null;
	return u.pathname;
}

export class Api {
	#art = new Map<string, Promise<string>>();
	/** How many views show each path's art now. */
	#artRefs = new Map<string, number>();
	// LMS resizes artwork on its single thread: don't flood it (HTTP/2 has no per-host cap).
	#artLimit = new Limiter(6);

	/**
	 * Told about every command's outcome as far as LMS's health goes: the error when the
	 * gateway said the music server is busy, null when LMS answered. Other failures say
	 * nothing either way.
	 */
	onHealth: (busy: ApiError | null) => void = () => {};
	/** Told when a command couldn't reach the server at all (see state/reachability). */
	onUnreachable: () => void = () => {};

	constructor(
		private session: SessionManager,
		private artCacheSize = 300,
	) {}

	/** Run an LMS command. `player` is a player id, or '' for library queries. */
	async rpc<T = Record<string, unknown>>(player: string, cmd: Command): Promise<T> {
		try {
			const result = await this.session.authed(async (token) => {
				const r = await callJson<{ result?: T }>('/v1/rpc', {
					method: 'POST',
					headers: { ...bearer(token), 'Content-Type': 'application/json' },
					body: JSON.stringify({ player, cmd }),
				});
				return (r?.result ?? {}) as T;
			});
			this.onHealth(null);
			return result;
		} catch (e) {
			if (isBusy(e)) this.onHealth(e);
			else if (isNetworkError(e)) this.onUnreachable();
			throw e;
		}
	}

	/**
	 * Artwork as a blob: URL. Art needs the bearer token, so it can't be a plain <img src>;
	 * fetching it also keeps tokens out of URLs. Cached, least recently used dropped first, but
	 * never while something on screen holds it (see `retainArt`): revoking a blob: URL an <img>
	 * still shows breaks it the next time the browser needs the image again.
	 */
	art(path: string): Promise<string> {
		const hit = this.#art.get(path);
		if (hit) {
			this.#art.delete(path);
			this.#art.set(path, hit); // most recently used last
			return hit;
		}
		const p = this.artBlob(path).then((b) => URL.createObjectURL(b));
		// Don't cache failures (unless a newer request has replaced this one meanwhile).
		p.catch(() => this.#art.get(path) === p && this.#art.delete(path));
		this.#art.set(path, p);
		this.#trim(path);
		return p;
	}

	/**
	 * Keep `path`'s blob: URL alive while it is shown; call the returned function when it no
	 * longer is (calling it again does nothing).
	 */
	retainArt(path: string): () => void {
		this.#artRefs.set(path, (this.#artRefs.get(path) ?? 0) + 1);
		let done = false;
		return () => {
			if (done) return;
			done = true;
			const n = (this.#artRefs.get(path) ?? 0) - 1;
			if (n > 0) this.#artRefs.set(path, n);
			else this.#artRefs.delete(path);
			this.#trim();
		};
	}

	/** Drop the least recently used art nobody holds until the cache fits (keeping `keep`). */
	#trim(keep?: string) {
		if (this.#art.size <= this.artCacheSize) return;
		for (const [path, p] of this.#art) {
			if (this.#art.size <= this.artCacheSize) break;
			if (path === keep || this.#artRefs.has(path)) continue;
			this.#art.delete(path);
			p.then((u) => URL.revokeObjectURL(u), () => {});
		}
	}

	/** Artwork as an image Blob (not cached here): e.g. to keep with offline downloads. */
	artBlob(path: string): Promise<Blob> {
		const url = artUrl(path);
		// The bearer token goes with this request: never let a path step out of /v1/art/.
		if (!url) return Promise.reject(new ApiError(400, 'bad art path'));
		return this.session.authed((token) =>
			this.#artLimit.run(async () => {
				const resp = await fetch(url, { headers: bearer(token), cache: 'default' });
				if (!resp.ok) throw new ApiError(resp.status, `art HTTP ${resp.status}`);
				const type = resp.headers.get('content-type') ?? '';
				if (!/^image\//.test(type)) throw new ApiError(415, 'not an image');
				const blob = await resp.blob();
				return blob.type ? blob : new Blob([blob], { type: type.split(';')[0]!.trim() });
			}),
		);
	}

	/**
	 * An artist's biography and photo, or an album's review (GET /v1/info/…), unchecked: see
	 * $lib/info/info for the checks. Rejects for a bad id without asking.
	 */
	info(kind: 'artist' | 'album', id: number): Promise<unknown> {
		if (!Number.isSafeInteger(id) || id <= 0 || id > 999_999_999_999) return Promise.reject(new ApiError(400, 'bad id'));
		return this.session.authed((token) => callJson<unknown>(`/v1/info/${kind}/${id}`, { headers: bearer(token) }));
	}

	clearArt() {
		for (const p of this.#art.values()) p.then((u) => URL.revokeObjectURL(u), () => {});
		this.#art.clear();
	}
}
