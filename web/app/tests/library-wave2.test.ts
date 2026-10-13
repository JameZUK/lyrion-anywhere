// Wave 2 part 2, library: tappable credits and contributor pages in a role ("Appears as"),
// Library's type chips, album order, grid or list (remembered), the A–Z fast scroller (pointer,
// keyboard and screen reader) and browse-list swipes with their alternatives.
import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { LIBRARY_KEY } from '$lib/library/prefs';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const params: Record<string, string | undefined> = {};
vi.mock('$app/state', () => ({ page: { params, url: new URL('http://app.example/#/') } }));
const fake = await import('./helpers/fake-app.svelte');
const TrackCredits = (await import('$lib/components/TrackCredits.svelte')).default;
const ArtistPage = (await import('../src/routes/artist/[id]/[[role]]/+page.svelte')).default;
const LibraryPage = (await import('../src/routes/library/+page.svelte')).default;
const AlbumBrowser = (await import('$lib/components/AlbumBrowser.svelte')).default;
const AzScroller = (await import('$lib/components/AzScroller.svelte')).default;
const AlbumList = (await import('$lib/components/AlbumList.svelte')).default;
const Chips = (await import('$lib/components/Chips.svelte')).default;

const P = '02:00:00:00:00:01';
const HOSTILE = '<img src=x onerror=alert(1)>';
type Cmd = (string | number)[];
const sent = (word: string): Cmd[] => fake.calls.filter((c) => c.cmd[0] === word).map((c) => c.cmd);
const settle = async () => {
	for (let i = 0; i < 6; i++) {
		await tick();
		await new Promise((r) => setTimeout(r, 0));
	}
};

beforeAll(() => {
	globalThis.IntersectionObserver = class {
		observe() {}
		disconnect() {}
		unobserve() {}
		takeRecords() {
			return [];
		}
		root = null;
		rootMargin = '';
		thresholds = [];
	} as unknown as typeof IntersectionObserver;
});

beforeEach(() => {
	fake.calls.length = 0;
	fake.app.playerId = P;
	fake.app.players = [{ playerid: P, name: 'Test player' }];
	fake.handler.rpc = async () => ({});
	for (const k of Object.keys(params)) delete params[k];
	localStorage.clear();
});

// --- 3.1 credits ------------------------------------------------------------------------------

test('track credits list every role, each name its own link to that role', async () => {
	fake.handler.rpc = async (_p, c) =>
		c[0] === 'titles'
			? { titles_loop: [{ id: 9, composer: 'Ada Example, Bo Placeholder', composer_ids: '31,32', artist: 'The Sample Band', artist_ids: '12', conductor: HOSTILE, conductor_ids: '40', band: 'Odd, Name, Here', band_ids: '1,2' }] }
			: {};
	const { container, getByRole } = render(TrackCredits, { track: 9 });
	await waitFor(() => expect(container.querySelector('[data-credits]')).not.toBeNull());
	expect(sent('titles')[0]).toEqual(['titles', 0, 1, 'tags:AS', 'track_id:9']);
	const links = [...container.querySelectorAll('a')].map((a) => [a.textContent, a.getAttribute('href')]);
	expect(links).toEqual([
		['The Sample Band', '#/artist/12/artist'],
		['Ada Example', '#/artist/31/composer'],
		['Bo Placeholder', '#/artist/32/composer'],
		[HOSTILE, '#/artist/40/conductor'],
	]);
	// Each link says its role to screen readers.
	expect(getByRole('link', { name: 'Ada Example, composer' })).toBeTruthy();
	// Names that can't be paired with ids are plain text, never a link.
	expect(container.textContent).toContain('Odd, Name, Here');
	expect(container.querySelectorAll('img, script').length).toBe(0);
	expect(container.querySelector('h3')?.textContent).toBe('Credits');
});

test('credits that fail to load say so quietly', async () => {
	fake.handler.rpc = async () => {
		throw new Error('refused');
	};
	const { container } = render(TrackCredits, { track: 9 });
	await waitFor(() => expect(container.textContent).toContain('Credits aren’t available'));
});

test('an artist page shows "Appears as" chips with counts; roles with none are hidden', async () => {
	params.id = '12';
	fake.handler.rpc = async (_p, c) => {
		const role = c.find((x) => String(x).startsWith('role_id:'));
		if (c[0] === 'albums' && c[2] === 1) return { count: { 'role_id:ALBUMARTIST': 4, 'role_id:COMPOSER': 12, 'role_id:CONDUCTOR': 3 }[String(role)] ?? 0 };
		if (c[0] === 'albums') return { albums_loop: [{ id: 1, album: 'First', artist: 'Invented Artist' }] };
		return {};
	};
	const { container, getByRole } = render(ArtistPage);
	await waitFor(() => expect(container.querySelector('[data-appears-as]')).not.toBeNull());
	await waitFor(() => expect(container.querySelectorAll('[data-appears-as] a').length).toBe(3));
	const chips = [...container.querySelectorAll('[data-appears-as] a')].map((a) => [a.textContent?.trim(), a.getAttribute('href')]);
	expect(chips).toEqual([
		['Album artist (4)', '#/artist/12/albumartist'],
		['Composer (12)', '#/artist/12/composer'],
		['Conductor (3)', '#/artist/12/conductor'],
	]);
	expect(getByRole('navigation', { name: 'Appears as' })).toBeTruthy();
	expect(getByRole('link', { name: 'Composer: 12 albums' })).toBeTruthy();
	// The default view is unchanged: all their albums.
	expect(sent('albums').find((c) => c[2] === 500)).toEqual(['albums', 0, 500, 'tags:aljyS', 'sort:yearalbum', 'artist_id:12']);
	expect(container.querySelector('h1')?.textContent).toBe('Invented Artist');
});

test('only album artist: no "Appears as" row', async () => {
	params.id = '12';
	fake.handler.rpc = async (_p, c) => (c[0] === 'albums' && c[2] === 1 ? { count: c.includes('role_id:ALBUMARTIST') ? 4 : 0 } : { albums_loop: [] });
	const { container } = render(ArtistPage);
	await settle();
	expect(container.querySelector('[data-appears-as]')).toBeNull();
});

test('a contributor page in a role: their albums in that role, named as LMS credits them', async () => {
	params.id = '31';
	params.role = 'composer';
	fake.handler.rpc = async (_p, c) => {
		if (c[0] === 'titles') return { titles_loop: [{ composer: 'Ada Example, Bo Placeholder', composer_ids: '31,32' }] };
		if (c[0] === 'albums' && c[2] === 1) return { count: c.includes('role_id:COMPOSER') ? 2 : 0 };
		if (c[0] === 'albums') return { albums_loop: [{ id: 1, album: 'Sonatas', artist: 'Someone Else' }] };
		return {};
	};
	const { container } = render(ArtistPage);
	await waitFor(() => expect(container.querySelector('h1')?.textContent).toContain('Ada Example'));
	expect(container.querySelector('h1')?.textContent).toBe('Ada Example · Composer');
	expect(sent('albums').find((c) => c[2] === 500)).toEqual(['albums', 0, 500, 'tags:aljyS', 'sort:yearalbum', 'artist_id:31', 'role_id:COMPOSER']);
	expect(sent('titles')[0]).toEqual(['titles', 0, 1, 'tags:AS', 'artist_id:31', 'role_id:COMPOSER']);
	await waitFor(() => expect(container.querySelector('[aria-current="page"]')?.textContent?.trim()).toBe('Composer (2)'));
	expect([...container.querySelectorAll('[data-appears-as] a')].map((a) => a.textContent?.trim())).toContain('All albums');
	// No "Play all" (that would play every role).
	expect(container.textContent).not.toContain('Play all');
});

test('a role word that is not a role is refused without asking LMS for albums', async () => {
	params.id = '31';
	params.role = 'producer;DROP';
	const { container } = render(ArtistPage);
	await settle();
	expect(container.textContent).toContain('Not a role.');
	expect(sent('albums').some((c) => c[2] === 500)).toBe(false);
	expect(sent('titles')).toEqual([]);
});

// --- 3.2 chips, sort, view, A–Z --------------------------------------------------------------

test('Library chips are tabs: arrows move and choose; the choice is remembered', async () => {
	fake.handler.rpc = async () => ({ count: 0 });
	const first = render(LibraryPage);
	const tabs = () => [...first.container.querySelectorAll<HTMLButtonElement>('[role="tab"][data-chip]')];
	expect(tabs().map((t) => t.textContent?.trim())).toEqual(['Albums', 'Artists', 'Playlists', 'Favourites']);
	// No Downloaded chip on the web.
	expect(first.container.textContent).not.toContain('Downloaded');
	expect(tabs()[0]!.getAttribute('aria-selected')).toBe('true');
	expect(tabs().filter((t) => t.tabIndex === 0)).toHaveLength(1);
	tabs()[0]!.focus();
	await fireEvent.keyDown(tabs()[0]!, { key: 'ArrowRight' });
	await settle();
	expect(tabs()[1]!.getAttribute('aria-selected')).toBe('true');
	expect(document.activeElement).toBe(tabs()[1]);
	expect(first.container.querySelector('[role="tabpanel"]')?.getAttribute('data-library-panel')).toBe('artists');
	await fireEvent.keyDown(tabs()[1]!, { key: 'End' });
	await settle();
	expect(first.container.querySelector('[data-library-panel]')?.getAttribute('data-library-panel')).toBe('favourites');
	expect(JSON.parse(localStorage.getItem(LIBRARY_KEY)!).chip).toBe('favourites');
	first.unmount();
	const again = render(LibraryPage);
	expect(again.container.querySelector('[data-library-panel]')?.getAttribute('data-library-panel')).toBe('favourites');
	again.unmount();
});

test('album order and grid or list are remembered; sorts offered are the gateway’s', async () => {
	fake.handler.rpc = async (_p, c) => (c[0] === 'albums' ? { count: 1, albums_loop: [{ id: 5, album: 'Five', artist: 'Invented' }] } : {});
	const first = render(LibraryPage);
	await waitFor(() => expect(first.container.querySelector('[data-pos="0"]')).not.toBeNull());
	const select = first.container.querySelector<HTMLSelectElement>('[data-library-panel] select')!;
	expect([...select.options].map((o) => o.textContent)).toEqual(['Recently added', 'A–Z', 'Artist', 'Year']);
	await fireEvent.change(select, { target: { value: 'yearalbum' } });
	await waitFor(() => expect(sent('albums').at(-1)).toContain('sort:yearalbum'));
	const list = first.container.querySelector<HTMLButtonElement>('[data-view="list"]')!;
	expect(list.getAttribute('aria-pressed')).toBe('false');
	await fireEvent.click(list);
	await settle();
	expect(list.getAttribute('aria-pressed')).toBe('true');
	expect(first.container.querySelector('[data-row-more]')).not.toBeNull();
	expect(JSON.parse(localStorage.getItem(LIBRARY_KEY)!)).toEqual({ chip: 'albums', sort: 'yearalbum', view: 'list' });
	first.unmount();
	fake.calls.length = 0;
	const again = render(LibraryPage);
	await waitFor(() => expect(sent('albums').length).toBeGreaterThan(0));
	expect(sent('albums')[0]).toContain('sort:yearalbum');
	await waitFor(() => expect(again.container.querySelector('[data-row-more]')).not.toBeNull());
	again.unmount();
});

test('sorted A–Z, the index comes with the first page and a letter jumps straight to its offset', async () => {
	fake.handler.rpc = async (_p, c) => {
		if (c[0] !== 'albums') return {};
		const from = Number(c[1]);
		const n = Number(c[2]);
		const rows = Array.from({ length: Math.min(n, 300 - from) }, (_, i) => ({ id: from + i + 1, album: `Album ${from + i + 1}` }));
		return { count: 300, albums_loop: rows, ...(String(c[3]).endsWith('Z') ? { indexList: [['A', 100], ['B', 150], ['C', 50]] } : {}) };
	};
	const { container } = render(AlbumBrowser, { sort: 'album', az: true });
	await waitFor(() => expect(container.querySelector('[data-az]')).not.toBeNull());
	expect(sent('albums')[0]).toEqual(['albums', 0, 120, 'tags:aljySZ', 'sort:album']);
	const letters = [...container.querySelectorAll<HTMLButtonElement>('[data-az] button')];
	expect(letters.map((b) => b.getAttribute('aria-label'))).toEqual(['A: 100 albums', 'B: 150 albums', 'C: 50 albums']);
	// C starts at 250: beyond what is loaded, so the list starts again there.
	await fireEvent.click(letters[2]!);
	await waitFor(() => expect(container.querySelector('[data-pos="250"]')).not.toBeNull());
	expect(sent('albums').at(-1)).toEqual(['albums', 250, 120, 'tags:aljyS', 'sort:album']);
	expect(container.querySelector('[data-pos="0"]')).toBeNull();
	await settle();
	expect(document.activeElement).toBe(container.querySelector('[data-pos="250"] a'));
	expect(container.querySelector('[role="status"]')?.textContent).toBe('Showing albums from C');
	// …and can page back towards the start.
	await fireEvent.click(container.querySelector('[data-earlier]')!);
	await waitFor(() => expect(container.querySelector('[data-pos="130"]')).not.toBeNull());
	expect(sent('albums').at(-1)).toEqual(['albums', 130, 120, 'tags:aljyS', 'sort:album']);
	// Within what is loaded: no new request.
	const before = sent('albums').length;
	await fireEvent.click(letters[2]!);
	await settle();
	expect(sent('albums').length).toBe(before);
	expect(document.activeElement).toBe(container.querySelector('[data-pos="250"] a'));
});

test('no index for other orders', async () => {
	fake.handler.rpc = async () => ({ count: 1, albums_loop: [{ id: 1, album: 'x' }], indexList: [['A', 1], ['B', 1]] });
	const { container } = render(AlbumBrowser, { sort: 'new', az: true });
	await settle();
	expect(sent('albums')[0]).toEqual(['albums', 0, 120, 'tags:aljyS', 'sort:new']);
	expect(container.querySelector('[data-az]')).toBeNull();
});

test('the scroller works from the keyboard: one tab stop, arrows move with a bubble, Enter jumps', async () => {
	const jumps: string[] = [];
	const index = [
		{ key: 'A', count: 3, offset: 0 },
		{ key: 'B', count: 2, offset: 3 },
		{ key: 'C', count: 5, offset: 5 },
	];
	const { container } = render(AzScroller, { index, onjump: (e: { key: string }) => jumps.push(e.key), noun: 'artists' });
	const bar = container.querySelector('[role="toolbar"]')!;
	expect(bar.getAttribute('aria-label')).toBe('Jump to a letter in the artists');
	const buttons = () => [...container.querySelectorAll<HTMLButtonElement>('[role="toolbar"] button')];
	expect(buttons().map((b) => b.tabIndex)).toEqual([0, -1, -1]);
	buttons()[0]!.focus();
	await fireEvent.keyDown(buttons()[0]!, { key: 'ArrowDown' });
	await tick();
	expect(document.activeElement).toBe(buttons()[1]);
	expect(buttons().map((b) => b.tabIndex)).toEqual([-1, 0, -1]);
	expect(container.querySelector('[data-az-bubble]')?.textContent).toBe('B');
	await fireEvent.keyDown(buttons()[1]!, { key: 'End' });
	await tick();
	expect(document.activeElement).toBe(buttons()[2]);
	await fireEvent.click(buttons()[2]!); // Enter or Space on a button
	expect(jumps).toEqual(['C']);
	expect(container.querySelector('[role="status"]')?.textContent).toBe('Showing artists from C');
});

test('dragging along the strip shows the letter under the finger and jumps on release', async () => {
	const jumps: string[] = [];
	const index = Array.from({ length: 4 }, (_, i) => ({ key: 'ABCD'[i]!, count: 1, offset: i }));
	const { container } = render(AzScroller, { index, onjump: (e: { key: string }) => jumps.push(e.key) });
	const bar = container.querySelector<HTMLElement>('[role="toolbar"]')!;
	bar.getBoundingClientRect = () => ({ top: 0, height: 100, left: 0, width: 44, right: 44, bottom: 100, x: 0, y: 0, toJSON() {} }) as DOMRect;
	await fireEvent.pointerDown(bar, { pointerId: 1, pointerType: 'touch', clientY: 10 });
	expect(container.querySelector('[data-az-bubble]')?.textContent).toBe('A');
	await fireEvent.pointerMove(bar, { pointerId: 1, pointerType: 'touch', clientY: 60 });
	expect(container.querySelector('[data-az-bubble]')?.textContent).toBe('C');
	await fireEvent.pointerUp(bar, { pointerId: 1, pointerType: 'touch', clientY: 60 });
	expect(jumps).toEqual(['C']);
	expect(container.querySelector('[data-az-bubble]')).toBeNull();
	// A cancelled drag does nothing.
	await fireEvent.pointerDown(bar, { pointerId: 2, pointerType: 'touch', clientY: 90 });
	await fireEvent.pointerCancel(bar, { pointerId: 2, pointerType: 'touch' });
	expect(jumps).toEqual(['C']);
});

test('a scroller with one letter or none is not shown', () => {
	const { container } = render(AzScroller, { index: [{ key: 'A', count: 3, offset: 0 }], onjump: () => {} });
	expect(container.querySelector('[role="toolbar"]')).toBeNull();
});

test('generic chips: Home, End and wrap-around; one tab stop', async () => {
	let value = 'b';
	const r = render(Chips, { items: [{ value: 'a', label: 'A' }, { value: 'b', label: 'B' }, { value: 'c', label: 'C' }], value, label: 'Pick', onchange: (v: string) => (value = v) });
	const tab = r.container.querySelectorAll<HTMLButtonElement>('[role="tab"]');
	expect([...tab].map((t) => t.tabIndex)).toEqual([-1, 0, -1]);
	await fireEvent.keyDown(tab[1]!, { key: 'Home' });
	expect(value).toBe('a');
	await r.rerender({ value: 'a' });
	await fireEvent.keyDown(tab[0]!, { key: 'ArrowLeft' });
	expect(value).toBe('c');
	await fireEvent.keyDown(tab[0]!, { key: 'x' });
	expect(value).toBe('c');
});

// --- browse-list swipes and their alternatives -----------------------------------------------

const albums = [
	{ id: 7, album: 'Seven', artist: 'Invented' },
	{ id: 8, album: HOSTILE, artist: HOSTILE },
];
const enqueued = () => sent('playlistcontrol');

test('album list: swipe right plays next, swipe left adds to the queue (touch only)', async () => {
	const { container } = render(AlbumList, { albums });
	const row = container.querySelector<HTMLElement>('[data-pos="0"] .row')!;
	row.getBoundingClientRect = () => ({ top: 0, height: 60, left: 0, width: 300, right: 300, bottom: 60, x: 0, y: 0, toJSON() {} }) as DOMRect;
	const swipe = async (dx: number, type = 'touch') => {
		await fireEvent.pointerDown(row, { pointerId: 1, pointerType: type, clientX: 150, clientY: 10 });
		await fireEvent.pointerMove(row, { pointerId: 1, pointerType: type, clientX: 150 + dx, clientY: 12 });
		await fireEvent.pointerUp(row, { pointerId: 1, pointerType: type, clientX: 150 + dx, clientY: 12 });
		await settle();
	};
	await swipe(140);
	expect(enqueued().at(-1)).toEqual(['playlistcontrol', 'cmd:insert', 'album_id:7']);
	await swipe(-140);
	expect(enqueued().at(-1)).toEqual(['playlistcontrol', 'cmd:add', 'album_id:7']);
	// Too short: springs back.
	await swipe(30);
	expect(enqueued()).toHaveLength(2);
	// A mouse drag is not a swipe.
	await swipe(200, 'mouse');
	expect(enqueued()).toHaveLength(2);
	// Mostly vertical: a scroll.
	await fireEvent.pointerDown(row, { pointerId: 3, pointerType: 'touch', clientX: 150, clientY: 10 });
	await fireEvent.pointerMove(row, { pointerId: 3, pointerType: 'touch', clientX: 160, clientY: 120 });
	await fireEvent.pointerUp(row, { pointerId: 3, pointerType: 'touch', clientX: 260, clientY: 130 });
	expect(enqueued()).toHaveLength(2);
});

test('album list: every swipe has a menu item and a key', async () => {
	const { container, getAllByRole } = render(AlbumList, { albums });
	const link = container.querySelector<HTMLAnchorElement>('[data-pos="0"] a')!;
	expect(link.getAttribute('aria-keyshortcuts')).toBe('N A');
	await fireEvent.keyDown(link, { key: 'n' });
	expect(enqueued().at(-1)).toEqual(['playlistcontrol', 'cmd:insert', 'album_id:7']);
	await fireEvent.keyDown(link, { key: 'A' });
	expect(enqueued().at(-1)).toEqual(['playlistcontrol', 'cmd:add', 'album_id:7']);
	// With a modifier, the key is the browser's.
	await fireEvent.keyDown(link, { key: 'a', ctrlKey: true });
	expect(enqueued()).toHaveLength(2);
	const more = container.querySelector<HTMLButtonElement>('[data-pos="1"] [data-row-more]')!;
	expect(more.getAttribute('aria-label')).toBe(`More options for ${HOSTILE}`);
	await fireEvent.click(more);
	await tick();
	await new Promise((r) => setTimeout(r, 0));
	const items = getAllByRole('menuitem');
	expect(items.map((i) => i.textContent?.replace(/[NA]$/, '').trim())).toEqual(['Play next', 'Add to queue']);
	expect(document.activeElement).toBe(items[0]);
	await fireEvent.keyDown(items[0]!, { key: 'ArrowDown' });
	expect(document.activeElement).toBe(items[1]);
	await fireEvent.click(items[1]!);
	expect(enqueued().at(-1)).toEqual(['playlistcontrol', 'cmd:add', 'album_id:8']);
	// Escape closes it and returns focus.
	await fireEvent.click(more);
	await tick();
	await new Promise((r) => setTimeout(r, 0));
	await fireEvent.keyDown(getAllByRole('menuitem')[0]!, { key: 'Escape' });
	expect(document.activeElement).toBe(more);
	expect(container.querySelectorAll('img, script').length).toBe(0);
});
