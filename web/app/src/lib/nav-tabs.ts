// The app's four main destinations, the same on phones (the bottom tab bar), at the top of
// the desktop sidebar, and in the Android app: Home, Library, Search and Queue. Now playing
// is not a tab: it opens from the mini player (a tap, or a swipe up). Settings sits behind
// the gear at the top of Home, so it counts as part of Home here.

export type MainTab = 'home' | 'library' | 'search' | 'queue';

export const MAIN_TABS: readonly { id: MainTab; href: string; label: string; icon: string }[] = [
	{ id: 'home', href: '#/home', label: 'Home', icon: 'shelves' },
	{ id: 'library', href: '#/library', label: 'Library', icon: 'library' },
	{ id: 'search', href: '#/search', label: 'Search', icon: 'search' },
	{ id: 'queue', href: '#/queue', label: 'Queue', icon: 'playnext' },
];

/** Each tab's pages, by the first segment of the route. */
const SECTIONS: Record<MainTab, readonly string[]> = {
	home: ['home', 'stations', 'settings', 'rooms'],
	library: [
		'library',
		'album',
		'albums',
		'artist',
		'artists',
		'genre',
		'genres',
		'year',
		'years',
		'folder',
		'compilations',
		'random',
		'filter',
		'new',
		'playlist',
		'playlists',
		'favourites',
		'apps',
		'browse',
		'plugin',
		'downloads',
	],
	search: ['search', 'search-on'],
	queue: ['queue'],
};

/**
 * Which tab a route belongs to (e.g. `/album/3` is under Library), or null for pages that
 * belong to none: Now playing (`/`), car mode, alarms and the launch shortcuts.
 */
export function tabFor(path: unknown): MainTab | null {
	if (typeof path !== 'string') return null;
	const first = path.replace(/^#/, '').replace(/[?#].*$/, '').split('/').filter(Boolean)[0];
	if (!first) return null;
	for (const t of MAIN_TABS) if (SECTIONS[t.id].includes(first)) return t.id;
	return null;
}
