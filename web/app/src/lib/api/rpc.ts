// LMS commands through the gateway's allow-list (POST /v1/rpc) and artwork (GET /v1/art).
import { ApiError, bearer, callJson } from './http';
import type { SessionManager } from './session';

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
	// LMS resizes artwork on its single thread: don't flood it (HTTP/2 has no per-host cap).
	#artLimit = new Limiter(6);

	constructor(
		private session: SessionManager,
		private artCacheSize = 300,
	) {}

	/** Run an LMS command. `player` is a player id, or '' for library queries. */
	rpc<T = Record<string, unknown>>(player: string, cmd: Command): Promise<T> {
		return this.session.authed(async (token) => {
			const r = await callJson<{ result?: T }>('/v1/rpc', {
				method: 'POST',
				headers: { ...bearer(token), 'Content-Type': 'application/json' },
				body: JSON.stringify({ player, cmd }),
			});
			return (r?.result ?? {}) as T;
		});
	}

	/**
	 * Artwork as a blob: URL. Art needs the bearer token, so it can't be a plain <img src>;
	 * fetching it also keeps tokens out of URLs. Cached, oldest dropped first.
	 */
	art(path: string): Promise<string> {
		const hit = this.#art.get(path);
		if (hit) {
			this.#art.delete(path);
			this.#art.set(path, hit); // most recently used last
			return hit;
		}
		const url = artUrl(path);
		// The bearer token goes with this request: never let a path step out of /v1/art/.
		if (!url) return Promise.reject(new ApiError(400, 'bad art path'));
		const p = this.session.authed((token) => this.#artLimit.run(async () => {
			const resp = await fetch(url, { headers: bearer(token), cache: 'force-cache' });
			if (!resp.ok) throw new ApiError(resp.status, `art HTTP ${resp.status}`);
			const type = resp.headers.get('content-type') ?? '';
			if (!/^image\//.test(type)) throw new ApiError(415, 'not an image');
			return URL.createObjectURL(await resp.blob());
		}));
		p.catch(() => this.#art.delete(path)); // don't cache failures
		this.#art.set(path, p);
		while (this.#art.size > this.artCacheSize) {
			const [oldest, old] = this.#art.entries().next().value!;
			this.#art.delete(oldest);
			old.then((u) => URL.revokeObjectURL(u), () => {});
		}
		return p;
	}

	clearArt() {
		for (const p of this.#art.values()) p.then((u) => URL.revokeObjectURL(u), () => {});
		this.#art.clear();
	}
}
