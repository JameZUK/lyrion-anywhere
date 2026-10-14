// What Library shows (Wave 2 §3.2; one model since Wave 3.5): the section, the album order
// and grid or list. Kept per device in local storage (never sent anywhere); storage that is
// blocked or holds something odd gives the defaults.
import type { BrowseSort } from '$lib/components/AlbumBrowser.svelte';

/** Library's sections, in order: one list (Wave 3.5), each shown in place. */
export type LibraryChip =
	| 'albums'
	| 'artists'
	| 'playlists'
	| 'favourites'
	| 'filters'
	| 'genres'
	| 'years'
	| 'compilations'
	| 'folders'
	| 'apps'
	| 'downloads';
export type AlbumView = 'grid' | 'list';

export const CHIPS: readonly LibraryChip[] = ['albums', 'artists', 'playlists', 'favourites', 'filters', 'genres', 'years', 'compilations', 'folders', 'apps', 'downloads'];
export const CHIP_LABELS: Record<LibraryChip, string> = {
	albums: 'Albums',
	artists: 'Artists',
	playlists: 'Playlists',
	favourites: 'Favourites',
	filters: 'Smart playlists',
	genres: 'Genres',
	years: 'Years',
	compilations: 'Compilations',
	folders: 'Folders',
	apps: 'Apps & radio',
	downloads: 'Downloads',
};

/**
 * Album orders Library offers. "Recently played" is the user's own (their listening history
 * on the gateway), never LMS's `recentlyplayed`, which is everyone's. "Recently added" is what
 * New music was, and "Random" what Random albums was (Wave 3.5: sorts, not pages of their own).
 */
export const LIBRARY_SORTS: readonly BrowseSort[] = ['new', 'played', 'album', 'artistalbum', 'yearalbum', 'random'];

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

/**
 * Open Library at a section (and, for albums, an order): what the old pages of their own
 * (`/albums`, `/artists`, `/new`, `/random`) now redirect to. Remembered like a choice made there.
 */
export function openSection(chip: LibraryChip, sort?: BrowseSort, storage: (Pick<Storage, 'getItem'> & Pick<Storage, 'setItem'>) | undefined = local()) {
	const now = loadLibraryPrefs(storage);
	saveLibraryPrefs({ ...now, chip, ...(sort ? { sort } : {}) }, storage);
}
