// The document title for each route ("Search – Lyrion Anywhere"). SvelteKit's route announcer
// reads the title after each navigation, so screen readers hear where they are. Detail pages
// (an album, an artist) add their item's name once they know it.
import { onDestroy } from 'svelte';

export const APP_NAME = 'Lyrion Anywhere';

const SECTIONS: Record<string, string> = {
	'': 'Now playing',
	home: 'Home',
	library: 'Library',
	new: 'New music',
	albums: 'Albums',
	album: 'Album',
	artists: 'Artists',
	artist: 'Artist',
	playlists: 'Playlists',
	playlist: 'Playlist',
	favourites: 'Favourites',
	apps: 'Apps & radio',
	browse: 'Apps & radio',
	plugin: 'Apps & radio',
	search: 'Search',
	queue: 'Queue',
	downloads: 'Downloads',
	settings: 'Settings',
	alarms: 'Alarms',
	stations: 'Stations',
	genres: 'Genres',
	genre: 'Genre',
	years: 'Years',
	year: 'Year',
	folder: 'Music folder',
	compilations: 'Compilations',
	random: 'Random albums',
};

/** The section a route path (e.g. `/album/7`, from the hash) belongs to. */
export function sectionTitle(path: string): string {
	const first = path.replace(/^#/, '').replace(/[?#].*$/, '').split('/').filter(Boolean)[0] ?? '';
	return SECTIONS[first] ?? APP_NAME;
}

/** The document title for `path`, with a detail page's item name when known. */
export function documentTitle(path: string, detail = ''): string {
	const name = detail.replace(/\s+/g, ' ').trim().slice(0, 120);
	const section = sectionTitle(path);
	const lead = name || section;
	return lead === APP_NAME ? APP_NAME : `${lead} – ${APP_NAME}`;
}

class PageTitle {
	/** The item a detail page shows (an album's title…), or ''. */
	detail = $state('');
}
export const pageTitle = new PageTitle();

/** For detail pages: name the item in the title while the page is shown. */
export function useDetailTitle(name: () => string) {
	$effect(() => {
		pageTitle.detail = name();
	});
	onDestroy(() => {
		pageTitle.detail = '';
	});
}
