// The library search: what was typed and what was found. It lives outside the page, so going
// to a result and back finds the search as it was. Answers that arrive out of order (a slow
// one for an earlier term) are dropped: only the latest term's answer is ever shown.
import type { SearchResult } from '$lib/lms/types';

/** Shortest term searched for. */
export const MIN_TERM = 2;
/** Typing pause before searching, in ms. */
export const DEBOUNCE_MS = 250;

export class SearchState {
	/** What is in the box. */
	term = $state('');
	result = $state<SearchResult | null>(null);
	/** The term `result` is for. */
	shownFor = $state('');
	busy = $state(false);
	error = $state('');
	#seq = 0;
	#timer: ReturnType<typeof setTimeout> | undefined;

	constructor(
		private ask: (term: string) => Promise<SearchResult>,
		private delay = DEBOUNCE_MS,
	) {}

	/** The box changed: search for it after a short pause. */
	set(term: string) {
		this.term = term;
		const q = term.trim();
		clearTimeout(this.#timer);
		// Anything still on its way is for an earlier term now.
		const mine = ++this.#seq;
		if (q.length < MIN_TERM) {
			this.result = null;
			this.shownFor = '';
			this.busy = false;
			this.error = '';
			return;
		}
		if (q === this.shownFor && this.result) {
			this.busy = false;
			return;
		}
		this.#timer = setTimeout(() => void this.#run(q, mine), this.delay);
	}

	async #run(q: string, mine: number) {
		if (mine !== this.#seq) return;
		this.busy = true;
		try {
			const r = await this.ask(q);
			if (mine !== this.#seq) return;
			this.result = r;
			this.shownFor = q;
			this.error = '';
		} catch (e) {
			if (mine === this.#seq) this.error = (e as Error).message;
		} finally {
			if (mine === this.#seq) this.busy = false;
		}
	}

	/** Forget the search (e.g. another user signed in). */
	reset() {
		clearTimeout(this.#timer);
		this.#seq++;
		this.term = '';
		this.result = null;
		this.shownFor = '';
		this.busy = false;
		this.error = '';
	}
}
