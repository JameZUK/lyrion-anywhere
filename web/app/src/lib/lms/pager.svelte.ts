// A list LMS sends a page at a time (genres, artists, folders): load more as it scrolls,
// start again when what is listed changes, and never show an entry twice.

export interface Page<T> {
	rows: T[];
	count?: number;
}

export class Pager<T> {
	items = $state<T[]>([]);
	total = $state<number | null>(null);
	error = $state('');
	#loading = false;
	#generation = 0;
	#fetch: (from: number) => Promise<Page<T>>;
	#key: (t: T) => string | number | null;

	/** `key` names an entry; entries without one (null) are left out. */
	constructor(fetch: (from: number) => Promise<Page<T>>, key: (t: T) => string | number | null) {
		this.#fetch = fetch;
		this.#key = key;
	}

	get done(): boolean {
		return this.total !== null && this.items.length >= this.total;
	}

	/** Start again, optionally with a new query. */
	reset(fetch?: (from: number) => Promise<Page<T>>) {
		if (fetch) this.#fetch = fetch;
		this.#generation++;
		this.#loading = false;
		this.items = [];
		this.total = null;
		this.error = '';
		return this.more();
	}

	async more() {
		if (this.#loading || this.done) return;
		this.#loading = true;
		const g = this.#generation;
		try {
			const r = await this.#fetch(this.items.length);
			if (g !== this.#generation) return;
			const seen = new Set(this.items.map(this.#key));
			const fresh = (Array.isArray(r.rows) ? r.rows : []).filter((x) => {
				const k = x === null || x === undefined ? null : this.#key(x);
				return k !== null && !seen.has(k) && !!seen.add(k);
			});
			this.items = [...this.items, ...fresh];
			const count = typeof r.count === 'number' && r.count >= 0 ? r.count : this.items.length;
			// A page with nothing new ends the list, so a confused server can't keep us asking.
			this.total = fresh.length ? Math.max(count, this.items.length) : this.items.length;
		} catch (e) {
			if (g === this.#generation) this.error = (e as Error).message;
		} finally {
			if (g === this.#generation) this.#loading = false;
		}
	}
}

/** Load more when `el` comes near the viewport. Returns a cleanup function. */
export function whenNear(el: Element | undefined, load: () => unknown, margin = '800px'): (() => void) | undefined {
	if (!el || typeof IntersectionObserver === 'undefined') return;
	const io = new IntersectionObserver((e) => e.some((x) => x.isIntersecting) && void load(), { rootMargin: margin });
	io.observe(el);
	return () => io.disconnect();
}
