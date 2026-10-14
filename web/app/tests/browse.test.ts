// The My Music browse pages (New music, Random albums, genres, years, artists by role, the
// music folder, virtual libraries): what they ask LMS for, that hostile answers stay inert
// text, and that the paging and the remembered library choice cope with odd input.
import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { Pager } from '$lib/lms/pager.svelte';
import { folderRow } from '$lib/lms/folder';
import { loadChoice, parseLibraries, saveChoice } from '$lib/state/library.svelte';

const { choose } = await import('./helpers/menu');
vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const params: Record<string, string | undefined> = {};
vi.mock('$app/state', () => ({ page: { params, url: new URL('http://app.example/#/') } }));
const fake = await import('./helpers/fake-app.svelte');
const { library } = await import('$lib/state/library.svelte');
const AlbumBrowser = (await import('$lib/components/AlbumBrowser.svelte')).default;
const NewMusic = (await import('../src/routes/new/+page.svelte')).default;
const RandomAlbums = (await import('../src/routes/random/+page.svelte')).default;
const Artists = (await import('../src/routes/artists/+page.svelte')).default;
const Folder = (await import('../src/routes/folder/[[id]]/+page.svelte')).default;
const Genres = (await import('../src/routes/genres/+page.svelte')).default;
const Years = (await import('../src/routes/years/+page.svelte')).default;
const LibraryPage = (await import('../src/routes/library/+page.svelte')).default;

const P = '02:00:00:00:00:01';
const HOSTILE = '<img src=x onerror=alert(1)>';
type Cmd = (string | number)[];
const sent = (word: string): Cmd[] => fake.calls.filter((c) => c.cmd[0] === word).map((c) => c.cmd);

function memoryStore(): Storage {
	const m = new Map<string, string>();
	return {
		getItem: (k: string) => m.get(k) ?? null,
		setItem: (k: string, v: string) => void m.set(k, v),
		removeItem: (k: string) => void m.delete(k),
		clear: () => m.clear(),
		key: () => null,
		get length() {
			return m.size;
		},
	};
}

function assertInert(container: HTMLElement) {
	expect(container.querySelectorAll('script, img, iframe, object, embed').length).toBe(0);
	for (const el of container.querySelectorAll('*')) {
		for (const attr of el.attributes) {
			expect(attr.name.startsWith('on'), `${el.tagName} has ${attr.name}`).toBe(false);
			if (attr.name === 'href') expect(attr.value).toMatch(/^#\//);
		}
	}
}

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

beforeEach(async () => {
	fake.calls.length = 0;
	fake.app.playerId = P;
	fake.handler.rpc = async () => ({});
	for (const k of Object.keys(params)) delete params[k];
	library.stop();
});

// --- paging -------------------------------------------------------------------------------

test('the pager adds pages, skips repeats and stops when nothing new comes', async () => {
	const pages = [
		{ rows: [{ id: 1 }, { id: 2 }], count: 10 },
		{ rows: [{ id: 2 }, { id: 3 }, { id: null }, null], count: 10 },
		{ rows: [{ id: 3 }], count: 10 },
	];
	let n = 0;
	const p = new Pager<{ id: number | null } | null>(async () => pages[n++]!, (x) => x?.id ?? null);
	await p.reset();
	expect(p.items.map((x) => x?.id)).toEqual([1, 2]);
	expect(p.done).toBe(false);
	await p.more();
	expect(p.items.map((x) => x?.id)).toEqual([1, 2, 3]);
	await p.more();
	// A page of repeats: the list ends rather than being asked for again and again.
	expect(p.done).toBe(true);
	expect(p.total).toBe(3);
	await p.more();
	expect(n).toBe(3);
});

test('the pager ignores an answer for a list it has since left', async () => {
	let release!: (v: { rows: { id: number }[] }) => void;
	const p = new Pager<{ id: number }>(() => new Promise((r) => (release = r)), (x) => x.id);
	const first = p.reset();
	const old = release;
	const second = p.reset(async () => ({ rows: [{ id: 9 }], count: 1 }));
	old({ rows: [{ id: 1 }] });
	await Promise.all([first, second]);
	expect(p.items).toEqual([{ id: 9 }]);
});

test('the pager reports errors and copes with odd answers', async () => {
	const p = new Pager<{ id: number }>(async () => {
		throw new Error('refused');
	}, (x) => x.id);
	await p.reset();
	expect(p.error).toBe('refused');
	const q = new Pager<{ id: number }>(async () => ({ rows: 'x' as unknown as { id: number }[], count: -5 }), (x) => x.id);
	await q.reset();
	expect(q.items).toEqual([]);
	expect(q.total).toBe(0);
});

// --- virtual libraries ----------------------------------------------------------------------

test('the chosen library is remembered per user, and only well-formed ids', () => {
	const s = memoryStore();
	saveChoice('alice', '1a2b3c4d', s);
	expect(loadChoice('alice', s)).toBe('1a2b3c4d');
	expect(loadChoice('bob', s)).toBeNull();
	saveChoice('alice', null, s);
	expect(loadChoice('alice', s)).toBeNull();
	// Tampered storage: ignored.
	s.setItem('anywhere.library.alice', '../x');
	expect(loadChoice('alice', s)).toBeNull();
	expect(() => saveChoice('alice', '/srv/music', s)).not.toThrow();
	expect(loadChoice('alice', s)).toBeNull();
	// Storage that refuses access (private windows, blocked site data): nothing breaks.
	const refusing = {
		getItem: () => {
			throw new Error('SecurityError');
		},
		setItem: () => {
			throw new Error('QuotaExceededError');
		},
		removeItem: () => {
			throw new Error('SecurityError');
		},
	};
	expect(loadChoice('alice', refusing)).toBeNull();
	expect(() => saveChoice('alice', '1a2b', refusing)).not.toThrow();
	expect(loadChoice('alice', undefined)).toBeNull();
	expect(loadChoice('', s)).toBeNull();
});

test('libraries LMS lists are kept only with well-formed ids', () => {
	expect(parseLibraries(undefined)).toEqual([]);
	expect(parseLibraries({})).toEqual([]);
	expect(
		parseLibraries({
			folder_loop: [
				{ id: '1a2b3c4d', name: 'Lossless (120)' },
				{ id: '1a2b3c4d', name: 'Again' },
				{ id: '../x', name: 'Bad' },
				{ id: 7 as unknown as string, name: 'Number' },
				{ id: 'abcdef12', name: HOSTILE },
				{ id: 'abcdef13' },
				null as unknown as { id: string },
			],
		}),
	).toEqual([
		{ id: '1a2b3c4d', name: 'Lossless (120)' },
		{ id: 'abcdef12', name: HOSTILE },
		{ id: 'abcdef13', name: 'Library' },
	]);
});

test('with no virtual libraries the picker is hidden and nothing is narrowed', async () => {
	fake.handler.rpc = async () => ({});
	await library.start('alice', (c) => fake.api.rpc('', c));
	expect(sent('libraries')).toEqual([['libraries']]);
	expect(library.available).toEqual([]);
	expect(library.id).toBeNull();
	const { container } = render(LibraryPage);
	expect(container.querySelector('.picker select')).toBeNull();
	expect(container.textContent).toContain('New music');
	expect(container.textContent).toContain('Music folder');
});

test('a chosen library narrows the album lists, and can be picked on the Library page', async () => {
	fake.handler.rpc = async (_p, cmd) =>
		cmd[0] === 'libraries' ? { folder_loop: [{ id: '1a2b3c4d', name: 'Lossless' }] } : { count: 0, albums_loop: [] };
	await library.start('alice', (c) => fake.api.rpc('', c));
	const page = render(LibraryPage);
	const select = page.container.querySelector<HTMLSelectElement>('.picker select')!;
	expect(select).not.toBeNull();
	await fireEvent.change(select, { target: { value: '1a2b3c4d' } });
	expect(library.id).toBe('1a2b3c4d');
	page.unmount();
	fake.calls.length = 0;
	const view = render(NewMusic);
	await waitFor(() => expect(sent('albums').length).toBe(1));
	expect(sent('albums')[0]).toContain('library_id:1a2b3c4d');
	expect(sent('albums')[0]).toContain('sort:new');
	expect(view.container.textContent).toContain('Lossless');
	// An id LMS no longer lists is never sent.
	library.choose('ffffffff');
	expect(library.id).toBeNull();
});

// --- pages --------------------------------------------------------------------------------

test('New music lists recently added albums, newest first, with an empty state', async () => {
	fake.handler.rpc = async () => ({ count: 0, albums_loop: [] });
	const { container } = render(NewMusic);
	await waitFor(() => expect(container.textContent).toContain('Nothing has been added'));
	expect(sent('albums')[0]).toEqual(['albums', 0, 120, 'tags:aljyS', 'sort:new']);
	expect(container.querySelector('select')).toBeNull(); // a fixed order
});

test('album lists show hostile names as text and repeated albums once', async () => {
	fake.handler.rpc = async () => ({
		count: 3,
		albums_loop: [
			{ id: 1, album: HOSTILE, artist: HOSTILE, artwork_track_id: '../../x' },
			{ id: 1, album: 'Again' },
			{ id: 'x', album: 'Bad id' },
			{ id: 2, album: 'Two' },
		],
	});
	const { container } = render(AlbumBrowser, { sort: 'album' });
	await waitFor(() => expect(container.querySelectorAll('li').length).toBe(2));
	assertInert(container);
	expect(container.textContent).toContain(HOSTILE);
	expect(container.textContent).not.toContain('Again');
});

test('Random albums asks for one batch and shuffles again on request', async () => {
	fake.handler.rpc = async () => ({ count: 1000, albums_loop: [{ id: 5, album: 'Five' }] });
	const { getByText } = render(RandomAlbums);
	await waitFor(() => expect(sent('albums').length).toBe(1));
	expect(sent('albums')[0]).toEqual(['albums', 0, 60, 'tags:aljyS', 'sort:random']);
	await fireEvent.click(getByText('Shuffle again'));
	await waitFor(() => expect(sent('albums').length).toBe(2));
	expect(sent('albums')[1]).toEqual(['albums', 0, 60, 'tags:aljyS', 'sort:random']);
});

test('artists default to album artists, and other roles show only where the library has them', async () => {
	fake.handler.rpc = async (_p, cmd) => {
		const role = cmd.find((x) => String(x).startsWith('role_id:'));
		if (cmd[2] === 1) return { count: role === 'role_id:COMPOSER' ? 4 : 0, artists_loop: [] };
		return { count: 1, artists_loop: [{ id: 3, artist: HOSTILE }] };
	};
	const { container, getByText, queryByText } = render(Artists);
	await waitFor(() => expect(getByText('Composers')).toBeTruthy());
	expect(queryByText('Conductors')).toBeNull();
	expect(queryByText('Bands')).toBeNull();
	// The first page asks for LMS's A–Z index too (tags:Z).
	expect(sent('artists')[0]).toEqual(['artists', 0, 200, 'tags:Z', 'role_id:ALBUMARTIST']);
	assertInert(container);
	await fireEvent.click(getByText('All artists'));
	await waitFor(() => expect(sent('artists').at(-1)).toEqual(['artists', 0, 200, 'tags:Z']));
	await fireEvent.click(getByText('Composers'));
	await waitFor(() => expect(sent('artists').at(-1)).toEqual(['artists', 0, 200, 'tags:Z', 'role_id:COMPOSER']));
	await fireEvent.click(getByText('Album artists')); // back to the default for other tests
});

test('genres and years link to their albums, with empty states', async () => {
	fake.handler.rpc = async (_p, cmd) =>
		cmd[0] === 'genres'
			? { count: 2, genres_loop: [{ id: 12, genre: HOSTILE }, { id: '1;x', genre: 'Bad' }] }
			: { count: 3, years_loop: [{ year: 1999 }, { year: 0 }, { year: '<b>' }] };
	const g = render(Genres);
	await waitFor(() => expect(g.container.querySelectorAll('a[href="#/genre/12"]').length).toBe(1));
	assertInert(g.container);
	expect(g.container.textContent).not.toContain('Bad');
	g.unmount();
	const y = render(Years);
	await waitFor(() => expect(y.container.querySelectorAll('a:not([data-back])').length).toBe(2));
	expect([...y.container.querySelectorAll('a:not([data-back])')].map((a) => a.getAttribute('href'))).toEqual(['#/year/1999', '#/year/0']);
	expect(y.container.textContent).toContain('No year');
	expect(sent('years')[0]).toEqual(['years', 0, 500, 'hasAlbums:1']);
	y.unmount();
	fake.handler.rpc = async () => ({ count: 0 });
	const empty = render(Genres);
	await waitFor(() => expect(empty.container.textContent).toContain('No genres in the library.'));
});

// --- the music folder -----------------------------------------------------------------------

test('folder entries never show a path, and need real ids', () => {
	expect(folderRow({ id: 11, filename: 'Jazz', type: 'folder' })).toEqual({ kind: 'folder', id: 11, name: 'Jazz' });
	expect(folderRow({ id: 12, filename: '/srv/music/Jazz/01.flac', type: 'track', duration: '3:01' })).toMatchObject({
		kind: 'track',
		name: '01.flac',
		duration: '3:01',
	});
	expect(folderRow({ id: 13, title: 'C:\\Music\\x.mp3', type: 'track', coverid: '../x' })).toMatchObject({ name: 'x.mp3', cover: null });
	expect(folderRow({ id: 14, filename: 'AC/DC Live', type: 'folder' })?.name).toBe('AC/DC Live');
	expect(folderRow({ id: 15, type: 'track', duration: '<b>' })).toMatchObject({ name: 'Untitled', duration: '' });
	for (const bad of [{ id: -1 }, { id: '1,2' }, { id: '/srv' }, { id: 0 }, null, 'x', {}]) {
		expect(folderRow(bad as never)).toBeNull();
	}
});

test('the music folder browses by id and plays folders and tracks by id', async () => {
	params.id = '11';
	fake.handler.rpc = async (_p, cmd) =>
		cmd[0] === 'musicfolder'
			? {
					count: 3,
					folder_loop: [
						{ id: 21, filename: HOSTILE, type: 'folder' },
						{ id: 22, filename: '/srv/music/x/song.flac', title: 'Song', type: 'track', duration: '3:01' },
						{ id: 23, filename: 'cover.jpg', type: 'unknown' },
					],
				}
			: {};
	const { container, getByLabelText } = render(Folder);
	await waitFor(() => expect(container.querySelectorAll('li').length).toBe(3));
	expect(sent('musicfolder')[0]).toEqual(['musicfolder', 0, 200, 'folder_id:11', 'tags:cdt']);
	assertInert(container);
	expect(container.textContent).not.toContain('/srv');
	expect(container.querySelector('a[href="#/folder/21"]')).not.toBeNull();
	// Each row has one trailing control (Wave 3.5): its ⋯ menu.
	await choose(`More options for ${HOSTILE}`, 'Play', container);
	await choose('More options for Song', 'Add to queue', container);
	await fireEvent.click(getByLabelText('Play Song'));
	const plays = fake.calls.filter((c) => c.cmd[0] === 'playlistcontrol');
	expect(plays.map((c) => c.cmd)).toEqual([
		['playlistcontrol', 'cmd:load', 'folder_id:21'],
		['playlistcontrol', 'cmd:add', 'track_id:22'],
		['playlistcontrol', 'cmd:load', 'track_id:22'],
	]);
	expect(plays.every((c) => c.player === P)).toBe(true);
	for (const c of fake.calls) for (const a of c.cmd) expect(String(a)).not.toMatch(/url:|:\/\/|^\/|file:/);
});

test('a bad folder id in the address is refused without asking LMS', async () => {
	for (const bad of ['..%2F..', '-1', '1,2', 'abc']) {
		params.id = bad;
		fake.calls.length = 0;
		const { container, unmount } = render(Folder);
		expect(container.textContent).toContain('Not a folder.');
		expect(sent('musicfolder')).toEqual([]);
		unmount();
	}
	delete params.id;
	fake.handler.rpc = async () => ({ count: 0 });
	const top = render(Folder);
	await waitFor(() => expect(top.container.textContent).toContain('The music folder is empty.'));
	expect(sent('musicfolder')[0]).toEqual(['musicfolder', 0, 200, 'tags:cdt']);
});
