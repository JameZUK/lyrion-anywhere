// The user's ratings for the tracks on screen: fetched once per track and remembered for
// the last few hundred, and the user's rating style (stars or a heart).
import type { RatingStyle, RoutineApi, TrackRating } from '$lib/api/routine';

export const REMEMBERED = 300;

export class RatingsState {
	style = $state<RatingStyle>('stars');
	/** Bumped on every change, so views re-read. */
	version = $state(0);
	#cache = new Map<number, TrackRating>();
	#styleLoaded = false;

	constructor(private api: RoutineApi) {}

	get(track: number | null | undefined): TrackRating | null {
		void this.version;
		return track ? (this.#cache.get(track) ?? null) : null;
	}

	#keep(r: TrackRating) {
		this.#cache.delete(r.track);
		this.#cache.set(r.track, r);
		while (this.#cache.size > REMEMBERED) this.#cache.delete(this.#cache.keys().next().value as number);
		this.version++;
	}

	/** Fetch ratings for `tracks` not yet known (and the style, once). */
	async load(tracks: (number | null | undefined)[]) {
		if (!this.#styleLoaded) {
			this.#styleLoaded = true;
			try {
				this.style = (await this.api.style()).style;
			} catch {
				this.#styleLoaded = false;
			}
		}
		const want = tracks.filter((t): t is number => typeof t === 'number' && t > 0 && !this.#cache.has(t));
		if (!want.length) return;
		try {
			for (const r of await this.api.ratings(want)) this.#keep(r);
		} catch {
			/* offline or not rateable: shown as unrated */
		}
	}

	/** Rate `track` (0 clears). Shown at once; put back if the gateway refuses. */
	async rate(track: number, value: number): Promise<boolean> {
		const before = this.#cache.get(track);
		const mine = value > 0 ? value : null;
		this.#keep({ track, mine, shared: before?.shared ?? null, rating: mine ?? before?.shared ?? null });
		try {
			const r = await this.api.rate(track, value);
			if (r) this.#keep(r);
			return true;
		} catch {
			if (before) this.#keep(before);
			else {
				this.#cache.delete(track);
				this.version++;
			}
			return false;
		}
	}

	async setStyle(style: RatingStyle) {
		this.style = style;
		await this.api.setStyle(style);
	}

	async clearAll(): Promise<number> {
		const r = await this.api.clearRatings();
		this.#cache.clear();
		this.version++;
		return r.removed;
	}

	reset() {
		this.#cache.clear();
		this.#styleLoaded = false;
		this.version++;
	}
}
