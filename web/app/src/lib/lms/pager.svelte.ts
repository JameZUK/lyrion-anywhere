// A list LMS sends a page at a time (genres, artists, folders): load more as it scrolls,
// start again when what is listed changes, and never show an entry twice.
import { parseIndexList, type IndexEntry } from './azindex';

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

/** A page of a list that can also carry LMS's A–Z index (`indexList`, on the first page). */
export interface IndexedPage<T> extends Page<T> {
	indexList?: unknown;
}

/**
 * A long list (albums, artists) seen through a window that starts at `base`: it pages
 * onwards as it scrolls, can start again at any offset (an A–Z jump, without loading the
 * list up to it), and can then page back towards the start ("Show earlier").
 */
export class WindowPager<T> {
	items = $state<T[]>([]);
	/** Where `items` starts in the whole list. */
	base = $state(0);
	total = $state<number | null>(null);
	error = $state('');
	/** LMS's A–Z index, as sent with the first page (see azindex.ts). */
	index = $state<IndexEntry[]>([]);
	#loading = false;
	#generation = 0;
	#fetch: (from: number, n: number, first: boolean) => Promise<IndexedPage<T>>;
	#key: (t: T) => string | number | null;
	#page: number;
	#single: boolean;

	/**
	 * `fetch` loads `n` entries from `from` (`first` when an index is wanted). With `single`,
	 * the list is one batch (e.g. random albums) and never pages.
	 */
	constructor(fetch: (from: number, n: number, first: boolean) => Promise<IndexedPage<T>>, key: (t: T) => string | number | null, page = 120, single = false) {
		this.#fetch = fetch;
		this.#key = key;
		this.#page = Math.max(1, Math.floor(page));
		this.#single = single;
	}

	get done(): boolean {
		return this.total !== null && this.base + this.items.length >= this.total;
	}

	#fresh(rows: unknown, seen: Set<string | number | null>): T[] {
		return (Array.isArray(rows) ? (rows as T[]) : []).filter((x) => {
			const k = x === null || x === undefined ? null : this.#key(x);
			return k !== null && !seen.has(k) && !!seen.add(k);
		});
	}

	/** Start again from `at` (0 by default), optionally with a new query. */
	reset(fetch?: (from: number, n: number, first: boolean) => Promise<IndexedPage<T>>, at = 0) {
		if (fetch) this.#fetch = fetch;
		this.#generation++;
		this.#loading = false;
		this.items = [];
		this.base = Math.max(0, Math.floor(at));
		this.total = null;
		this.error = '';
		if (fetch || at === 0) this.index = [];
		return this.more();
	}

	/** Show the list from `offset`: within what is loaded, nothing to load; otherwise start there. */
	async jumpTo(offset: number): Promise<void> {
		if (!Number.isSafeInteger(offset) || offset < 0) return;
		if (offset >= this.base && offset < this.base + this.items.length) return;
		const keep = this.index;
		await this.reset(undefined, offset);
		if (!this.index.length) this.index = keep;
	}

	async more() {
		if (this.#loading || this.done) return;
		this.#loading = true;
		const g = this.#generation;
		const from = this.base + this.items.length;
		try {
			const r = await this.#fetch(from, this.#page, from === 0);
			if (g !== this.#generation) return;
			if (from === 0 && r && 'indexList' in r) this.index = parseIndexList(r.indexList);
			const fresh = this.#fresh(r?.rows, new Set(this.items.map(this.#key)));
			this.items = [...this.items, ...fresh];
			const end = this.base + this.items.length;
			const count = typeof r?.count === 'number' && Number.isSafeInteger(r.count) && r.count >= 0 ? r.count : end;
			// A page with nothing new ends the list, so a confused server can't keep us asking.
			this.total = this.#single ? end : fresh.length ? Math.max(count, end) : end;
		} catch (e) {
			if (g === this.#generation) this.error = (e as Error).message;
		} finally {
			if (g === this.#generation) this.#loading = false;
		}
	}

	/** Load the page before `base` (after a jump). */
	async earlier() {
		if (this.#loading || this.base <= 0) return;
		this.#loading = true;
		const g = this.#generation;
		const from = Math.max(0, this.base - this.#page);
		const n = this.base - from;
		try {
			const r = await this.#fetch(from, n, false);
			if (g !== this.#generation) return;
			const seen = new Set(this.items.map(this.#key));
			const fresh = this.#fresh(r?.rows, seen).slice(0, n);
			this.items = [...fresh, ...this.items];
			this.base = from === 0 || fresh.length === n ? from : Math.max(0, this.base - fresh.length);
			if (!fresh.length) this.base = 0;
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
