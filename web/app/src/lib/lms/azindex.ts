// The A–Z fast scroller's index (Wave 2 §3.2). LMS answers `albums … sort:album tags:Z` and
// `artists … tags:Z` with `indexList`: each first character and how many entries start with
// it, in list order. The running total before a letter is where that letter starts, so a
// letter jumps straight to its offset without loading the list up to it.

export interface IndexEntry {
	/** What is shown: the first character (or LMS's group mark, e.g. "#"). */
	key: string;
	count: number;
	/** Where the letter starts in the list. */
	offset: number;
}

/** Most letters kept (a hostile or odd answer can't build a huge scroller). */
export const MAX_KEYS = 120;

/** LMS's `indexList` as offsets; anything malformed is skipped, never thrown on. */
export function parseIndexList(raw: unknown): IndexEntry[] {
	if (!Array.isArray(raw)) return [];
	const out: IndexEntry[] = [];
	let offset = 0;
	for (const e of raw.slice(0, 10_000)) {
		if (!Array.isArray(e) || e.length < 2) continue;
		const [k, c] = e as unknown[];
		const count = typeof c === 'number' ? c : typeof c === 'string' && /^\d{1,9}$/.test(c) ? Number(c) : NaN;
		if (!Number.isSafeInteger(count) || count <= 0) continue;
		let key = typeof k === 'string' || typeof k === 'number' ? String(k) : '';
		// One character (a surrogate pair counts as one), printable.
		key = [...key.replace(/[\u0000-\u001f\u007f-\u009f]/g, '').trim()].slice(0, 1).join('');
		if (!key) key = '#';
		if (out.length && out[out.length - 1]!.key === key) {
			out[out.length - 1]!.count += count;
		} else if (out.length < MAX_KEYS) {
			out.push({ key, count, offset });
		} else {
			// Past the cap, later letters are folded into the last one kept.
			out[out.length - 1]!.count += count;
		}
		offset += count;
	}
	return out;
}

/** The letter the entry at `position` falls under, or null. */
export function keyAt(index: readonly IndexEntry[], position: number): IndexEntry | null {
	if (!Number.isFinite(position) || position < 0) return null;
	let found: IndexEntry | null = null;
	for (const e of index) {
		if (e.offset > position) break;
		found = e;
	}
	return found && position < found.offset + found.count ? found : null;
}

/** Which letter a pointer at `y` over a strip `top`…`top + height` points at (index into the list). */
export function keyFromPointer(count: number, top: number, height: number, y: number): number {
	if (count <= 0 || !Number.isFinite(y) || !(height > 0)) return 0;
	const i = Math.floor(((y - top) / height) * count);
	return Math.max(0, Math.min(count - 1, i));
}
