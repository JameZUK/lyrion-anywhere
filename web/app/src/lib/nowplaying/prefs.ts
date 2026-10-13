// How Now playing looks on this device (docs/plans/wave2-design.md §4.1–4.2): the layout (Art,
// Lyrics or Info), whether the visualiser is on and its style, and whether the queue panel
// shows beside it in wide windows. Kept in local storage (never sent anywhere); storage that
// is blocked or holds something odd gives the defaults.
import { STYLES, type VisualStyle } from '$lib/visual/draw';

export type Layout = 'art' | 'lyrics' | 'info';
export const LAYOUTS: readonly Layout[] = ['art', 'lyrics', 'info'];
export const LAYOUT_LABELS: Record<Layout, string> = { art: 'Art', lyrics: 'Lyrics', info: 'Info' };

export interface NowPlayingPrefs {
	layout: Layout;
	/** The visualiser: off unless chosen. */
	visualiser: boolean;
	style: VisualStyle;
	/** The queue beside Now playing in wide windows. */
	queuePanel: boolean;
}

export const DEFAULT_NOW_PLAYING: NowPlayingPrefs = { layout: 'art', visualiser: false, style: 'bars', queuePanel: true };
export const NOW_PLAYING_KEY = 'anywhere.nowplaying';

function local(): Storage | undefined {
	try {
		return globalThis.localStorage;
	} catch {
		return undefined;
	}
}

const pick = <T extends string>(v: unknown, from: readonly T[], fallback: T): T =>
	typeof v === 'string' && (from as readonly string[]).includes(v) ? (v as T) : fallback;
const flag = (v: unknown, fallback: boolean) => (typeof v === 'boolean' ? v : fallback);

export function loadNowPlaying(storage: Pick<Storage, 'getItem'> | undefined = local()): NowPlayingPrefs {
	try {
		const raw = storage?.getItem(NOW_PLAYING_KEY) ?? 'null';
		if (raw.length > 1024) return { ...DEFAULT_NOW_PLAYING };
		const v = JSON.parse(raw) as Record<string, unknown> | null;
		if (!v || typeof v !== 'object' || Array.isArray(v)) return { ...DEFAULT_NOW_PLAYING };
		return {
			layout: pick(v.layout, LAYOUTS, DEFAULT_NOW_PLAYING.layout),
			visualiser: flag(v.visualiser, DEFAULT_NOW_PLAYING.visualiser),
			style: pick(v.style, STYLES, DEFAULT_NOW_PLAYING.style),
			queuePanel: flag(v.queuePanel, DEFAULT_NOW_PLAYING.queuePanel),
		};
	} catch {
		return { ...DEFAULT_NOW_PLAYING };
	}
}

export function saveNowPlaying(p: NowPlayingPrefs, storage: Pick<Storage, 'setItem'> | undefined = local()) {
	try {
		storage?.setItem(NOW_PLAYING_KEY, JSON.stringify({ layout: p.layout, visualiser: p.visualiser, style: p.style, queuePanel: p.queuePanel }));
	} catch {
		/* storage blocked or full: the choice lasts until the page closes */
	}
}

/** The layout a number key picks (`1`, `2`, `3`), or null. */
export function layoutForKey(key: string): Layout | null {
	const i = ['1', '2', '3'].indexOf(key);
	return i < 0 ? null : LAYOUTS[i]!;
}

/** The next layout for a sideways swipe: right to left goes on, left to right goes back. */
export function swipeLayout(current: Layout, dx: number): Layout {
	const i = Math.max(0, LAYOUTS.indexOf(current));
	const to = dx < 0 ? Math.min(LAYOUTS.length - 1, i + 1) : Math.max(0, i - 1);
	return LAYOUTS[to]!;
}
