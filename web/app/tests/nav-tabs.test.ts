// The four main tabs (Home, Library, Search, Queue), the same on phones, at the top of the
// desktop sidebar and in the Android app; which tab each page belongs to.
import { readFileSync } from 'node:fs';
import { MAIN_TABS, SIDEBAR, tabFor } from '#lib/nav-tabs';

const layout = readFileSync(`${__dirname}/../src/routes/+layout.svelte`, 'utf8');

test('four tabs, in order, each with a label, an icon and a route', () => {
	expect(MAIN_TABS.map((t) => t.label)).toEqual(['Home', 'Library', 'Search', 'Queue']);
	expect(MAIN_TABS.map((t) => t.href)).toEqual(['#/home', '#/library', '#/search', '#/queue']);
	for (const t of MAIN_TABS) expect(t.icon).toBeTruthy();
	// No Playing or Settings tab: Now playing opens from the mini player, Settings from Home.
	expect(MAIN_TABS.some((t) => t.href === '#/' || t.href === '#/settings')).toBe(false);
});

test('pages light up the tab they belong to', () => {
	const cases: [string, string | null][] = [
		['/home', 'home'],
		['/stations', 'home'],
		['/settings', 'home'],
		['/settings/sound', 'home'],
		['/rooms', 'home'],
		['/library', 'library'],
		['/album/12', 'library'],
		['/albums', 'library'],
		['/artist/3', 'library'],
		['/artists', 'library'],
		['/genre/4', 'library'],
		['/genres', 'library'],
		['/years', 'library'],
		['/folder/9', 'library'],
		['/playlist/2', 'library'],
		['/favourites', 'library'],
		['/apps', 'library'],
		['/browse/radio', 'library'],
		['/plugin/x', 'library'],
		['/downloads', 'library'],
		['/new', 'library'],
		['/filter/6', 'library'],
		['/search', 'search'],
		['/search?q=blue', 'search'],
		['/search-on/spotty', 'search'],
		['/queue', 'queue'],
		['#/queue', 'queue'],
		// Now playing and the full-screen pages belong to no tab.
		['/', null],
		['', null],
		['/car', null],
		['/alarms', null],
		['/go/resume', null],
	];
	for (const [path, tab] of cases) expect([path, tabFor(path)]).toEqual([path, tab]);
});

test('look-alike and junk routes match nothing', () => {
	for (const p of ['/homes', '/albumz', '/queue-x', '/searching', '/../settings/..', '//', '/%2Fhome', 'javascript:alert(1)']) {
		expect([p, tabFor(p)]).toEqual([p, null]);
	}
	for (const v of [null, undefined, 3, {}, ['/home']]) expect(tabFor(v)).toBeNull();
});

test('the phone tab bar is built from the four, four columns wide', () => {
	expect(layout).toMatch(/const TABS = MAIN_TABS\.map/);
	expect(layout).toMatch(/grid-template-columns: repeat\(4, 1fr\)/);
	expect(layout).not.toMatch(/label: 'Playing'/);
});

test('the desktop sidebar is seven: the same four, then Now playing, Rooms and Settings (Wave 3.5)', () => {
	expect(layout).toMatch(/const NAV = SIDEBAR;/);
	expect(SIDEBAR.map((n) => n.label)).toEqual(['Home', 'Library', 'Search', 'Queue', 'Now playing', 'Rooms', 'Settings']);
	expect(SIDEBAR.slice(0, 4).map((n) => n.href)).toEqual(MAIN_TABS.map((t) => t.href));
	// Library's sections are not repeated in the sidebar.
	for (const gone of ['New music', 'Albums', 'Artists', 'Playlists', 'Favourites', 'Apps & radio', 'Downloads']) expect(SIDEBAR.map((n) => n.label)).not.toContain(gone);
	// Now playing has its own icon (never Home's house), and no two entries share one.
	expect(SIDEBAR.find((n) => n.label === 'Now playing')!.icon).toBe('nowplaying');
	expect(new Set(SIDEBAR.map((n) => n.icon)).size).toBe(SIDEBAR.length);
});

test('every page lights up exactly one sidebar entry, or none; an album lights up Library, not Albums', () => {
	const lit = (p: string) => SIDEBAR.filter((n) => n.match(p)).map((n) => n.label);
	expect(lit('/album/7')).toEqual(['Library']);
	expect(lit('/artist/3')).toEqual(['Library']);
	expect(lit('/downloads')).toEqual(['Library']);
	expect(lit('/apps')).toEqual(['Library']);
	expect(lit('/')).toEqual(['Now playing']);
	expect(lit('/rooms')).toEqual(['Rooms']);
	expect(lit('/settings/account')).toEqual(['Settings']);
	expect(lit('/home')).toEqual(['Home']);
	expect(lit('/queue')).toEqual(['Queue']);
	for (const p of ['/car', '/alarms', '/stats']) expect(lit(p).length).toBeLessThanOrEqual(1);
});
