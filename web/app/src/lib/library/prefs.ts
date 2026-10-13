// What Library shows (Wave 2 §3.2): the type chip, the album order and grid or list. Kept per
// device in local storage (never sent anywhere); storage that is blocked or holds something
// odd gives the defaults.
import type { BrowseSort } from '$lib/components/AlbumBrowser.svelte';

export type LibraryChip = 'albums' | 'artists' | 'playlists' | 'favourites' | 'filters';
export type AlbumView = 'grid' | 'list';

export const CHIPS: readonly LibraryChip[] = ['albums', 'artists', 'playlists', 'favourites', 'filters'];
export const CHIP_LABELS: Record<LibraryChip, string> = {
	albums: 'Albums',
	artists: 'Artists',
	playlists: 'Playlists',
	favourites: 'Favourites',
	filters: 'Filters',
};

/**
 * Album orders Library offers. "Recently played" is the user's own (their listening history
 * on the gateway), never LMS's `recentlyplayed`, which is everyone's.
 */
export const LIBRARY_SORTS: readonly BrowseSort[] = ['new', 'played', 'album', 'artistalbum', 'yearalbum'];

export interface LibraryPrefs {
	chip: LibraryChip;
	sort: BrowseSort;
	view: AlbumView;
}

export const DEFAULT_LIBRARY: LibraryPrefs = { chip: 'albums', sort: 'album', view: 'grid' };
export const LIBRARY_KEY = 'anywhere.library';

function local(): Storage | undefined {
	try {
		return globalThis.localStorage;
	} catch {
		return undefined;
	}
}

const pick = <T extends string>(v: unknown, from: readonly T[], fallback: T): T =>
	typeof v === 'string' && (from as readonly string[]).includes(v) ? (v as T) : fallback;

export function loadLibraryPrefs(storage: Pick<Storage, 'getItem'> | undefined = local()): LibraryPrefs {
	try {
		const v = JSON.parse(storage?.getItem(LIBRARY_KEY) ?? 'null') as Record<string, unknown> | null;
		if (!v || typeof v !== 'object') return { ...DEFAULT_LIBRARY };
		return {
			chip: pick(v.chip, CHIPS, DEFAULT_LIBRARY.chip),
			sort: pick(v.sort, LIBRARY_SORTS, DEFAULT_LIBRARY.sort),
			view: pick(v.view, ['grid', 'list'] as const, DEFAULT_LIBRARY.view),
		};
	} catch {
		return { ...DEFAULT_LIBRARY };
	}
}

export function saveLibraryPrefs(p: LibraryPrefs, storage: Pick<Storage, 'setItem'> | undefined = local()) {
	try {
		storage?.setItem(LIBRARY_KEY, JSON.stringify({ chip: p.chip, sort: p.sort, view: p.view }));
	} catch {
		/* storage blocked or full: the choice lasts until the page closes */
	}
}
