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
		const p = this.session.authed((token) => this.#artLimit.run(async () => {
			const resp = await fetch(`/v1/art/${path}`, { headers: bearer(token), cache: 'force-cache' });
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
