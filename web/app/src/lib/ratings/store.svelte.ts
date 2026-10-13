// The user's ratings for the tracks on screen: fetched once per track and remembered for
// the last few hundred, and the user's rating style (stars or a heart).
//
// Answers can arrive out of order (a slow fetch after a quick change, two quick changes), and
// a sign-out can land while requests are in flight: every answer is checked against what the
// store has done since it was asked, and stale ones are dropped.
import type { RatingStyle, RoutineApi, TrackRating } from '$lib/api/routine';

export const REMEMBERED = 300;

export class RatingsState {
	style = $state<RatingStyle>('stars');
	/** Bumped on every change, so views re-read. */
	version = $state(0);
	#cache = new Map<number, TrackRating>();
	#styleLoaded = false;
	/** Bumped by reset and clearAll: answers asked for before then are dropped. */
	#gen = 0;
	/** Bumped per track by every change the user makes: older answers for it are dropped. */
	#edits = new Map<number, number>();
	/** Bumped by every style change: an older style fetch is dropped. */
	#styleEdits = 0;
	/** Tracks being fetched now (not asked for twice). */
	#inFlight = new Set<number>();

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

	#edit(track: number): number {
		const n = (this.#edits.get(track) ?? 0) + 1;
		this.#edits.delete(track);
		this.#edits.set(track, n);
		while (this.#edits.size > REMEMBERED) this.#edits.delete(this.#edits.keys().next().value as number);
		return n;
	}

	/** Fetch ratings for `tracks` not yet known (and the style, once). */
	async load(tracks: (number | null | undefined)[]) {
		const gen = this.#gen;
		if (!this.#styleLoaded) {
			this.#styleLoaded = true;
			const edits = this.#styleEdits;
			try {
				const s = (await this.api.style()).style;
				if (gen === this.#gen && edits === this.#styleEdits) this.style = s;
			} catch {
				if (gen === this.#gen) this.#styleLoaded = false;
			}
		}
		if (gen !== this.#gen) return;
		const want = [
			...new Set(tracks.filter((t): t is number => typeof t === 'number' && t > 0 && !this.#cache.has(t) && !this.#inFlight.has(t))),
		];
		if (!want.length) return;
		const asked = new Map(want.map((t) => [t, this.#edits.get(t) ?? 0]));
		for (const t of want) this.#inFlight.add(t);
		try {
			const answers = await this.api.ratings(want);
			if (gen !== this.#gen) return; // reset (signed out) meanwhile
			for (const r of answers) {
				// Only tracks asked about, and only while the user hasn't changed them since.
				if (asked.has(r.track) && asked.get(r.track) === (this.#edits.get(r.track) ?? 0)) this.#keep(r);
			}
		} catch {
			/* offline or not rateable: shown as unrated */
		} finally {
			if (gen === this.#gen) for (const t of want) this.#inFlight.delete(t);
		}
	}

	/** Rate `track` (0 clears). Shown at once; put back if the gateway refuses. */
	async rate(track: number, value: number): Promise<boolean> {
		const gen = this.#gen;
		const before = this.#cache.get(track);
		const edit = this.#edit(track);
		const mine = value > 0 ? value : null;
		this.#keep({ track, mine, shared: before?.shared ?? null, rating: mine ?? before?.shared ?? null });
		const latest = () => gen === this.#gen && this.#edits.get(track) === edit;
		try {
			const r = await this.api.rate(track, value);
			// A newer change (or a sign-out) owns what is shown now.
			if (r && latest()) this.#keep(r);
			return true;
		} catch {
			if (!latest()) return false;
			if (before) this.#keep(before);
			else {
				this.#cache.delete(track);
				this.version++;
			}
			return false;
		}
	}

	/** Change the style; put back (and throws) if the gateway refuses. */
	async setStyle(style: RatingStyle) {
		const gen = this.#gen;
		const before = this.style;
		const edit = ++this.#styleEdits;
		this.style = style;
		try {
			await this.api.setStyle(style);
		} catch (e) {
			if (gen === this.#gen && edit === this.#styleEdits) this.style = before;
			throw e;
		}
	}

	async clearAll(): Promise<number> {
		const r = await this.api.clearRatings();
		this.#forget();
		return r.removed;
	}

	#forget() {
		this.#gen++;
		this.#cache.clear();
		this.#edits.clear();
		this.#inFlight.clear();
		this.version++;
	}

	/** Forget everything (sign-out, or another user signed in). */
	reset() {
		this.#forget();
		this.#styleLoaded = false;
		this.#styleEdits++;
		this.style = 'stars';
	}
}
