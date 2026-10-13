// Now playing on phones: the hero is a pager of Cover, Lyrics and Track info (the same three
// layouts as prefs.ts, under the names the phone shows). Pure helpers, tested on their own.
import { LAYOUTS, type Layout } from './prefs';

/** What each page of the pager is called (its dot's name, and its panel's). */
export const PAGE_LABELS: Record<Layout, string> = { art: 'Cover', lyrics: 'Lyrics', info: 'Track info' };

/** A sideways swipe must travel this far (px), and mostly sideways. */
export const PAGE_SWIPE_PX = 50;
/** A swipe up over the cover (touch or pen) peeks at the queue. */
export const PEEK_SWIPE_PX = 60;

export type PagerGesture = 'next' | 'previous' | 'peek' | null;

/** What a pointer that went down and came up `dx`, `dy` px away asks for. */
export function pagerGesture(dx: number, dy: number, pointerType: string): PagerGesture {
	if (!Number.isFinite(dx) || !Number.isFinite(dy)) return null;
	const ax = Math.abs(dx);
	const ay = Math.abs(dy);
	if (ax >= PAGE_SWIPE_PX && ax > ay * 1.5) return dx < 0 ? 'next' : 'previous';
	if (pointerType !== 'mouse' && dy <= -PEEK_SWIPE_PX && ay > ax * 1.5) return 'peek';
	return null;
}

/** The page a key moves to from `current` (←, →, Home, End), or null for any other key. */
export function pageForKey(current: Layout, key: string): Layout | null {
	const i = Math.max(0, LAYOUTS.indexOf(current));
	const last = LAYOUTS.length - 1;
	switch (key) {
		case 'ArrowRight':
			return LAYOUTS[Math.min(last, i + 1)]!;
		case 'ArrowLeft':
			return LAYOUTS[Math.max(0, i - 1)]!;
		case 'Home':
			return LAYOUTS[0]!;
		case 'End':
			return LAYOUTS[last]!;
		default:
			return null;
	}
}

/** The page next to `current` in a direction (stays put at either end). */
export function stepPage(current: Layout, by: 1 | -1): Layout {
	const i = Math.max(0, LAYOUTS.indexOf(current));
	return LAYOUTS[Math.max(0, Math.min(LAYOUTS.length - 1, i + by))]!;
}
