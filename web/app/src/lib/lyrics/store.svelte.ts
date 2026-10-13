// The playing track's lyrics: fetched once per track and remembered for the last few
// (no lyrics is remembered too, so a track without any isn't asked about again and again).
import { ApiError, bearer, callJson } from '$lib/api/http';
import { loadPeek, lyricsPath, sanitiseLyrics, savePeek, type Lyrics } from './lyrics';

export type Authed = <T>(fn: (token: string) => Promise<T>) => Promise<T>;

/** Tracks remembered. */
export const REMEMBERED = 50;
/** After a failed fetch, try again after these delays (while the track is still playing). */
export const RETRY_MS = [5_000, 20_000, 60_000];

export class LyricsState {
	/** The track the lyrics below are for. */
	trackId = $state<number | null>(null);
	lyrics = $state<Lyrics | null>(null);
	loading = $state(false);
	/** The small current line over Now playing (on unless switched off). */
	peek = $state(loadPeek());

	#cache = new Map<number, Lyrics | null>();
	#want: number | null = null;
	/** The wanted track whose last fetch failed (not "none": offline, busy). */
	#failed: number | null = null;
	#attempts = 0;
	#retry: ReturnType<typeof setTimeout> | undefined;
	/** Bumped by reset: answers asked for before it are dropped. */
	#gen = 0;

	constructor(private authed: Authed) {}

	setPeek(on: boolean) {
		this.peek = on;
		savePeek(on);
	}

	/** Show the lyrics for `id` (a library track), fetching them if needed. */
	async load(id: unknown) {
		const path = lyricsPath(id);
		const n = path ? (id as number) : null;
		// The same track again: nothing to do, unless its last fetch failed (then try now).
		if (n === this.#want && n !== this.#failed) return;
		if (n !== this.#want) this.#attempts = 0;
		clearTimeout(this.#retry);
		this.#failed = null;
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
		await this.#fetch(n, path);
	}

	async #fetch(n: number, path: string) {
		const gen = this.#gen;
		this.loading = true;
		let found: Lyrics | null = null;
		let failed = false;
		try {
			found = sanitiseLyrics(await this.authed((token) => callJson<unknown>(path, { headers: bearer(token) })));
		} catch (e) {
			// 404 is "none"; anything else (offline, busy) may succeed later, so isn't remembered.
			failed = !(e instanceof ApiError && e.status === 404);
		}
		if (gen !== this.#gen) return; // reset meanwhile (e.g. signed out)
		if (!failed) {
			this.#cache.delete(n);
			this.#cache.set(n, found);
			while (this.#cache.size > REMEMBERED) this.#cache.delete(this.#cache.keys().next().value as number);
		}
		if (this.#want !== n) return; // the track changed meanwhile
		this.lyrics = found;
		this.loading = false;
		if (!failed) return;
		// Try again by itself a few times while this track is still the one wanted (and at once
		// if it is asked for again).
		this.#failed = n;
		const delay = RETRY_MS[this.#attempts++];
		if (delay === undefined) return;
		this.#retry = setTimeout(() => {
			if (this.#want !== n || this.#failed !== n || gen !== this.#gen) return;
			this.#failed = null;
			void this.#fetch(n, path);
		}, delay);
	}

	/** Forget everything. */
	reset() {
		this.#gen++;
		clearTimeout(this.#retry);
		this.#failed = null;
		this.#attempts = 0;
		this.#cache.clear();
		this.#want = null;
		this.trackId = null;
		this.lyrics = null;
		this.loading = false;
	}
}
