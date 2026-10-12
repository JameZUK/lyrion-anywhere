// The playing track's lyrics: fetched once per track and remembered for the last few
// (no lyrics is remembered too, so a track without any isn't asked about again and again).
import { ApiError, bearer, callJson } from '$lib/api/http';
import { loadPeek, lyricsPath, sanitiseLyrics, savePeek, type Lyrics } from './lyrics';

export type Authed = <T>(fn: (token: string) => Promise<T>) => Promise<T>;

/** Tracks remembered. */
export const REMEMBERED = 50;

export class LyricsState {
	/** The track the lyrics below are for. */
	trackId = $state<number | null>(null);
	lyrics = $state<Lyrics | null>(null);
	loading = $state(false);
	/** The small current line over Now playing (on unless switched off). */
	peek = $state(loadPeek());

	#cache = new Map<number, Lyrics | null>();
	#want: number | null = null;

	constructor(private authed: Authed) {}

	setPeek(on: boolean) {
		this.peek = on;
		savePeek(on);
	}

	/** Show the lyrics for `id` (a library track), fetching them if needed. */
	async load(id: unknown) {
		const path = lyricsPath(id);
		const n = path ? (id as number) : null;
		if (n === this.#want) return;
		this.#want = n;
		this.trackId = n;
		if (n === null || !path) {
			this.lyrics = null;
			this.loading = false;
			return;
		}
		if (this.#cache.has(n)) {
			this.lyrics = this.#cache.get(n) ?? null;
			this.loading = false;
			return;
		}
		this.lyrics = null;
		this.loading = true;
		let found: Lyrics | null = null;
		let remember = true;
		try {
			found = sanitiseLyrics(await this.authed((token) => callJson<unknown>(path, { headers: bearer(token) })));
		} catch (e) {
			// 404 is "none"; anything else (offline, busy) may succeed later.
			remember = e instanceof ApiError && e.status === 404;
		}
		if (remember) {
			this.#cache.delete(n);
			this.#cache.set(n, found);
			while (this.#cache.size > REMEMBERED) this.#cache.delete(this.#cache.keys().next().value as number);
		}
		if (this.#want !== n) return; // the track changed meanwhile
		this.lyrics = found;
		this.loading = false;
	}

	/** Forget everything. */
	reset() {
		this.#cache.clear();
		this.#want = null;
		this.trackId = null;
		this.lyrics = null;
		this.loading = false;
	}
}
