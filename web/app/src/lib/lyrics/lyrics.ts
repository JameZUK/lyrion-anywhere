// Lyrics for Now playing (docs/plans/wave1-design.md §9): fetched from the gateway
// (GET /v1/lyrics/track/{id}, already parsed by the Rust core there), checked here as
// untrusted input, and looked up by playing position.

export interface LyricLine {
	/** When the line starts, ms (0 for plain lyrics). */
	t: number;
	text: string;
}

export interface Lyrics {
	source: 'tags' | 'plugin';
	synced: boolean;
	lines: LyricLine[];
}

/** Most lines, and characters in a line, accepted (the gateway keeps to the same limits). */
export const MAX_LINES = 4000;
export const MAX_LINE_CHARS = 500;

/** The gateway's answer, checked; null if it isn't lyrics. */
export function sanitiseLyrics(v: unknown): Lyrics | null {
	if (!v || typeof v !== 'object') return null;
	const o = v as Record<string, unknown>;
	if (o.source !== 'tags' && o.source !== 'plugin') return null;
	if (typeof o.synced !== 'boolean' || !Array.isArray(o.lines)) return null;
	const lines: LyricLine[] = [];
	for (const l of o.lines.slice(0, MAX_LINES)) {
		if (!l || typeof l !== 'object') continue;
		const { t, text } = l as Record<string, unknown>;
		if (typeof text !== 'string') continue;
		const ms = typeof t === 'number' && Number.isSafeInteger(t) && t >= 0 ? t : 0;
		lines.push({ t: o.synced ? ms : 0, text: text.slice(0, MAX_LINE_CHARS) });
	}
	if (o.synced) lines.sort((a, b) => a.t - b.t);
	if (!lines.some((l) => l.text.trim())) return null;
	return { source: o.source, synced: o.synced, lines };
}

/** The line playing at `ms`: the last one starting at or before it; -1 before the first line or for plain lyrics. */
export function lineAt(l: Lyrics | null, ms: number): number {
	if (!l?.synced || !Number.isFinite(ms)) return -1;
	let lo = 0;
	let hi = l.lines.length;
	while (lo < hi) {
		const mid = (lo + hi) >> 1;
		if (l.lines[mid]!.t <= ms) lo = mid + 1;
		else hi = mid;
	}
	return lo - 1;
}

/** A library track id for the URL: a positive integer, nothing else. */
export function lyricsPath(id: unknown): string | null {
	return typeof id === 'number' && Number.isSafeInteger(id) && id > 0 ? `/v1/lyrics/track/${id}` : null;
}

// --- the "lyrics peek" setting (on unless switched off; kept in this browser) -----------

const PEEK_KEY = 'anywhere.lyrics.peek';
type Store = Pick<Storage, 'getItem' | 'setItem'>;
function safeLocal(): Store | undefined {
	try {
		return globalThis.localStorage;
	} catch {
		return undefined;
	}
}

export function loadPeek(store: Store | undefined = safeLocal()): boolean {
	try {
		return store?.getItem(PEEK_KEY) !== '0';
	} catch {
		return true;
	}
}

export function savePeek(on: boolean, store: Store | undefined = safeLocal()) {
	try {
		store?.setItem(PEEK_KEY, on ? '1' : '0');
	} catch {
		/* not essential */
	}
}
