// The Home layout (docs/plans/wave1-design.md §7): which shelves, in what order, which are
// hidden. Saved per user on the gateway; these checks mirror the gateway's (gateway/src/
// home.rs), so a layout that fails here is never sent.
import { SHELF_TYPES, type HomeLayout, type HomeShelf, type ShelfType } from '#lib/api/me';

export const MAX_SHELVES = 24;
/** Shelves that name something by id: one of the user's stations, pinned menus or smart filters, or a genre. */
export const WITH_ID: readonly ShelfType[] = ['station', 'menu', 'genre', 'filter'];

export const SHELF_LABELS: Record<ShelfType, string> = {
	recent: 'Recently played',
	stations: 'Stations',
	new: 'New music',
	most: 'Most played',
	on_this_day: 'On this day',
	random: 'Random albums',
	favourites: 'Favourites',
	playlists: 'Playlists',
	station: 'Station',
	menu: 'Pinned menu',
	genre: 'Genre',
	filter: 'Smart playlist',
};

/** The Home every user starts with. */
export function defaultLayout(): HomeLayout {
	return {
		version: 1,
		shelves: (['recent', 'stations', 'new', 'most', 'on_this_day'] as const).map((type) => ({ type })),
	};
}

export const shelfKey = (s: HomeShelf) => (s.id === undefined ? s.type : `${s.type}:${s.id}`);

const validId = (v: unknown) => typeof v === 'number' && Number.isSafeInteger(v) && v >= 1 && v <= 999_999_999_999;

/** Why this shelf isn't acceptable, or null. */
export function shelfProblem(s: unknown): string | null {
	if (!s || typeof s !== 'object' || Array.isArray(s)) return 'each shelf is an object';
	const o = s as Record<string, unknown>;
	if (Object.keys(o).some((k) => !['type', 'id', 'hidden'].includes(k))) return 'unexpected field in a shelf';
	if (typeof o.type !== 'string' || !(SHELF_TYPES as readonly string[]).includes(o.type)) return 'unknown shelf type';
	const needsId = WITH_ID.includes(o.type as ShelfType);
	if (o.id !== undefined && !validId(o.id)) return 'bad id';
	if (needsId !== (o.id !== undefined)) return needsId ? 'this shelf needs an id' : 'this shelf takes no id';
	if (o.hidden !== undefined && typeof o.hidden !== 'boolean') return 'hidden must be true or false';
	return null;
}

/** Why this layout isn't acceptable, or null. */
export function layoutProblem(l: unknown): string | null {
	if (!l || typeof l !== 'object' || Array.isArray(l)) return 'a layout is an object';
	const o = l as Record<string, unknown>;
	if (o.version !== 1) return 'unsupported version';
	if (!Array.isArray(o.shelves)) return 'shelves missing';
	if (o.shelves.length > MAX_SHELVES) return 'too many shelves';
	const seen = new Set<string>();
	for (const s of o.shelves) {
		const p = shelfProblem(s);
		if (p) return p;
		const k = shelfKey(s as HomeShelf);
		if (seen.has(k)) return 'a shelf appears twice';
		seen.add(k);
	}
	return null;
}

/** A clean copy (only the fields the gateway takes; `hidden` only when true). */
export function clean(l: HomeLayout): HomeLayout {
	return {
		version: 1,
		shelves: l.shelves.map((s) => ({ type: s.type, ...(s.id !== undefined ? { id: s.id } : {}), ...(s.hidden ? { hidden: true } : {}) })),
	};
}

const withShelves = (l: HomeLayout, shelves: HomeShelf[]): HomeLayout => ({ version: 1, shelves });

/** Move the shelf at `from` to `to` (both clamped). */
export function move(l: HomeLayout, from: number, to: number): HomeLayout {
	const s = [...l.shelves];
	if (from < 0 || from >= s.length) return l;
	const t = Math.max(0, Math.min(s.length - 1, to));
	const [item] = s.splice(from, 1);
	s.splice(t, 0, item!);
	return withShelves(l, s);
}

export const moveUp = (l: HomeLayout, i: number) => move(l, i, i - 1);
export const moveDown = (l: HomeLayout, i: number) => move(l, i, i + 1);

export function toggleHidden(l: HomeLayout, i: number): HomeLayout {
	if (i < 0 || i >= l.shelves.length) return l;
	return withShelves(
		l,
		l.shelves.map((s, j) => (j === i ? { ...s, hidden: !s.hidden } : s)),
	);
}

export function remove(l: HomeLayout, i: number): HomeLayout {
	if (i < 0 || i >= l.shelves.length) return l;
	return withShelves(
		l,
		l.shelves.filter((_, j) => j !== i),
	);
}

/** Add a shelf at the end. Throws (with the reason) if the result wouldn't be valid. */
export function add(l: HomeLayout, shelf: HomeShelf): HomeLayout {
	const next = withShelves(l, [...l.shelves, shelf]);
	const p = layoutProblem(clean(next));
	if (p) throw new RangeError(p === 'a shelf appears twice' ? 'That shelf is already on Home.' : p);
	return next;
}

/** Shelf types that can be added that aren't on Home yet (without an id). */
export function addable(l: HomeLayout): ShelfType[] {
	const have = new Set(l.shelves.map(shelfKey));
	return SHELF_TYPES.filter((t) => WITH_ID.includes(t) || !have.has(t));
}
