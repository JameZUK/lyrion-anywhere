// "Search on <plugin>" (Wave 2 §3.3, U7): which apps offer a search (found from their own top
// menus, nothing hard-coded), that the text goes only into the search box's `search:` value,
// that hostile menus and text are refused, the per-session directory, the rows under the
// library results, and the results page. Invented app names only.
import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import type { SBMenu } from '#lib/lms/slimbrowse';
import { resolveAction } from '#lib/lms/slimbrowse';
import { loadRecents } from '#lib/search/recents';
import { CONCURRENCY, MAX_APPS, SearchDirectory, candidates, cleanQuery, discover, isSearchBox, searchBox, searchCommand, searchOnHref, signature } from '#lib/search/plugins';

const { choose } = await import('./helpers/menu');
vi.mock('#lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const params: Record<string, string | undefined> = {};
vi.mock('$app/state', () => ({ page: { params, url: new URL('http://app.example/#/') } }));
const fake = await import('./helpers/fake-app.svelte');
const { search } = await import('#lib/state/search');
const { pluginSearch } = await import('#lib/state/plugin-search');
const SearchPage = (await import('../src/routes/search/+page.svelte')).default;
const ResultsPage = (await import('../src/routes/search-on/[app]/[...q]/+page.svelte')).default;

const P = '02:00:00:00:00:01';
const HOSTILE = '<img src=x onerror=alert(1)>';
type Cmd = (string | number)[];

/** An app's top menu with a search box (its own action) and something to play. */
const topMenu = (app: string): SBMenu => ({
	count: 2,
	item_loop: [
		{ text: 'Search', type: 'search', input: { len: 1 }, actions: { go: { cmd: [app, 'items'], params: { item_id: '0', search: '__TAGGEDINPUT__', menu: app } } } },
		{ text: 'A track', type: 'audio', actions: { play: { cmd: [app, 'playlist', 'play'], params: { item_id: '1' } } } },
	],
});
/** A search box through the menu's base actions, as some apps send it. */
const baseMenu = (app: string): SBMenu => ({
	base: { actions: { go: { cmd: [app, 'items'], params: { menu: app }, itemsParams: 'params' } } },
	item_loop: [
		{ text: 'Browse', type: 'link', params: { item_id: '1' } },
		{ text: 'Find', type: 'search', params: { item_id: '2', search: '__INPUT__' } },
	],
});
const noSearch: SBMenu = { item_loop: [{ text: 'Browse', actions: { go: { cmd: ['x', 'items'], params: { item_id: '1' } } } }] };

const settle = async () => {
	for (let i = 0; i < 8; i++) {
		await tick();
		await new Promise((r) => setTimeout(r, 0));
	}
};

describe('finding search boxes', () => {
	test('an item offering a menu with the text as `search:` is a search box', () => {
		const box = searchBox(topMenu('examplemusic'))!;
		expect(box).toEqual({ cmd: ['examplemusic', 'items'], params: { item_id: '0', search: '__TAGGEDINPUT__', menu: 'examplemusic' }, menu: true, nextWindow: undefined });
		expect(searchBox(baseMenu('exampleradio'))?.params).toEqual({ menu: 'exampleradio', item_id: '2', search: '__INPUT__' });
		expect(searchBox(noSearch)).toBeNull();
		expect(searchBox({})).toBeNull();
		expect(searchBox(null)).toBeNull();
	});

	test('anything else is not used: text into another parameter, into several, or a command', () => {
		const one = (go: unknown, extra: Record<string, unknown> = {}): SBMenu => ({ item_loop: [{ text: 'Search', type: 'search', ...extra, actions: { go } }] });
		for (const m of [
			// Typed text going somewhere other than `search:`.
			one({ cmd: ['x', 'items'], params: { q: '__TAGGEDINPUT__' } }),
			one({ cmd: ['x', 'items'], params: { search: 'prefix __TAGGEDINPUT__' } }),
			// Into two parameters.
			one({ cmd: ['x', 'items'], params: { search: '__TAGGEDINPUT__', title: '__INPUT__' } }),
			// A command that runs (not a menu of results).
			one({ cmd: ['x', 'add'], params: { search: '__TAGGEDINPUT__' } }, { nextWindow: 'nowPlaying' }),
			// Malformed words, URLs and odd keys are never resolved at all.
			one({ cmd: ['../x', 'items'], params: { search: '__TAGGEDINPUT__' } }),
			one({ cmd: ['x', 'items'], params: { search: '__TAGGEDINPUT__', url: 'file:///srv/music' } }),
			one({ cmd: ['x', 'items'], params: { search: '__TAGGEDINPUT__', 'bad key': '1' } }),
			one('nonsense'),
			// A search box with nothing to open.
			{ item_loop: [{ text: 'Search', type: 'search', input: {} }] },
		]) {
			expect(searchBox(m)).toBeNull();
		}
		expect(isSearchBox(null)).toBe(false);
	});

	test('robust on arbitrary menus: never throws', () => {
		const junk: unknown[] = [null, 1, 'x', [], { item_loop: 'x' }, { item_loop: [null, 1, 'a', [], { actions: null }, { actions: { go: { cmd: [1, 2] } } }] }, { base: { actions: { go: 5 } }, item_loop: [{ params: {} }] }];
		for (let i = 0; i < 200; i++) {
			const item: Record<string, unknown> = {};
			for (const k of ['text', 'type', 'input', 'actions', 'params']) if (Math.random() < 0.6) item[k] = junk[Math.floor(Math.random() * junk.length)];
			junk.push({ item_loop: [item] });
		}
		for (const m of junk) expect(() => searchBox(m as SBMenu)).not.toThrow();
	});
});

describe('the command sent', () => {
	const box = searchBox(topMenu('examplemusic'))!;
	const page = { start: 0, count: 100 };

	test('the text goes only into `search:`, as one value', () => {
		const c = searchCommand(box, 'x item_id:9 menu:1', page);
		expect(c).toEqual(['examplemusic', 'items', 0, 100, 'item_id:0', 'search:x item_id:9 menu:1', 'menu:examplemusic']);
		expect(c.filter((w) => String(w).startsWith('item_id:'))).toEqual(['item_id:0']);
		expect(c.filter((w) => String(w).startsWith('search:'))).toHaveLength(1);
	});

	test('control characters are blanked, the text bounded, empty text refused', () => {
		expect(searchCommand(box, ' abba\u0000\ngold\u007f ', page)).toContain('search:abba  gold');
		const long = searchCommand(box, 'y'.repeat(1000), page).find((w) => String(w).startsWith('search:')) as string;
		expect(long.length).toBe('search:'.length + 200);
		for (const empty of ['', '   ', '\n\t', '\u0000']) expect(() => searchCommand(box, empty, page)).toThrow(RangeError);
		expect(cleanQuery(undefined)).toBe('');
		expect(cleanQuery(42)).toBe('');
	});

	test('only a search box: other actions are refused, never sent', () => {
		const play = resolveAction(topMenu('examplemusic'), topMenu('examplemusic').item_loop![1]!, 'play')!;
		expect(() => searchCommand(play, 'abc', page)).toThrow(RangeError);
		expect(() => searchCommand({ ...box, params: { ...box.params, q: '__INPUT__' } }, 'abc', page)).toThrow(RangeError);
		expect(() => searchCommand({ ...box, menu: false }, 'abc', page)).toThrow(RangeError);
	});

	test('the link carries the app and the text encoded', () => {
		expect(searchOnHref('examplemusic', 'AC/DC #1 ?')).toBe('#/search-on/examplemusic/AC%2FDC%20%231%20%3F');
	});
});

describe('discovery and the directory', () => {
	test('candidates: valid commands once each, bounded', () => {
		const many = Array.from({ length: 100 }, (_, i) => ({ cmd: `app${i}`, name: `App ${i}` }));
		expect(candidates([many]).length).toBe(MAX_APPS);
		expect(candidates([[{ cmd: 'a' }, { cmd: 'A' }, { cmd: '../x' }, { cmd: 3 as unknown as string }, {}], [{ cmd: 'a' }, { cmd: 'b' }]]).map((a) => a.cmd)).toEqual(['a', 'b']);
		expect(signature([{ cmd: 'a' }, { cmd: 'b' }])).toBe('a,b');
	});

	test('keeps apps with a search box, in order; one that fails is left out; a few at a time', async () => {
		let open = 0;
		let most = 0;
		const apps = ['one', 'two', 'three', 'four', 'five'].map((cmd) => ({ cmd, name: `Example ${cmd}` }));
		const found = await discover(apps, async (cmd) => {
			most = Math.max(most, ++open);
			await new Promise((r) => setTimeout(r, 1));
			open--;
			if (cmd === 'two') throw new Error('not available');
			return cmd === 'four' ? noSearch : topMenu(cmd);
		});
		expect(found.map((p) => p.cmd)).toEqual(['one', 'three', 'five']);
		expect(found[0]!.name).toBe('Example one');
		expect(most).toBeLessThanOrEqual(CONCURRENCY);
	});

	test('found once per session; again when the apps listed change; forgotten on reset', async () => {
		let lists = [[{ cmd: 'examplemusic', name: 'Example Music' }], [{ cmd: 'exampleradio', name: 'Example Radio' }]];
		const opened: string[] = [];
		const dir = new SearchDirectory({
			lists: async () => lists,
			open: async (_p, cmd) => {
				opened.push(cmd);
				return cmd === 'exampleradio' ? baseMenu(cmd) : topMenu(cmd);
			},
		});
		expect(await dir.get('')).toEqual([]);
		expect((await dir.get(P)).map((p) => p.name)).toEqual(['Example Music', 'Example Radio']);
		expect(opened).toEqual(['examplemusic', 'exampleradio']);
		await dir.get(P);
		expect(opened).toHaveLength(2); // cached
		// An app hidden from this account (the gateway leaves it out of the list): gone.
		lists = [[{ cmd: 'examplemusic', name: 'Example Music' }], []];
		expect((await dir.get(P)).map((p) => p.cmd)).toEqual(['examplemusic']);
		expect(opened).toHaveLength(3);
		dir.reset();
		await dir.get(P);
		expect(opened).toHaveLength(4);
	});

	test('find reads the app’s search box again; apps not listed (or invalid) are refused', async () => {
		const opened: string[] = [];
		const dir = new SearchDirectory({
			lists: async () => [[{ cmd: 'examplemusic', name: 'Example Music' }]],
			open: async (_p, cmd) => {
				opened.push(cmd);
				return topMenu(cmd);
			},
		});
		const hit = await dir.find(P, 'examplemusic');
		expect(hit?.name).toBe('Example Music');
		expect(hit?.search.params.search).toBe('__TAGGEDINPUT__');
		expect(opened).toEqual(['examplemusic', 'examplemusic']);
		await dir.find(P, 'examplemusic');
		expect(opened).toHaveLength(3); // read again every time
		expect(await dir.find(P, 'notlisted')).toBeNull();
		expect(await dir.find(P, '../etc')).toBeNull();
		expect(await dir.find('', 'examplemusic')).toBeNull();
		expect(opened.every((c) => c === 'examplemusic')).toBe(true);
	});
});

describe('on the Search page', () => {
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

	let apps: { cmd: string; name: string }[];
	beforeEach(() => {
		fake.calls.length = 0;
		fake.app.playerId = P;
		fake.session.current = { user: { username: 'ann', role: 'listener' }, device: { id: 1, name: 'Phone' } };
		localStorage.clear();
		search.reset();
		pluginSearch.reset();
		apps = [
			{ cmd: 'examplemusic', name: 'Example Music' },
			{ cmd: 'examplevideo', name: HOSTILE },
			{ cmd: 'exampleplain', name: 'Example Plain' },
		];
		fake.handler.rpc = async (_p, c) => {
			if (c[0] === 'search') return { contributors_count: 0, albums_count: 0, tracks_count: 0 };
			if (c[0] === 'playlists') return { count: 0, playlists_loop: [] };
			if (c[0] === 'apps') return { count: apps.length, appss_loop: apps };
			if (c[0] === 'radios') return { count: 1, radioss_loop: [{ cmd: 'exampleradio', name: 'Example Radio' }] };
			if (c[0] === 'exampleplain') return noSearch;
			if (c[1] === 'items') return topMenu(String(c[0]));
			return {};
		};
	});

	async function typeTerm(container: HTMLElement, term: string) {
		const box = container.querySelector<HTMLInputElement>('[data-search-input]')!;
		await fireEvent.input(box, { target: { value: term } });
		await waitFor(() => expect(container.querySelector('[role="status"]')!.textContent).toMatch(/result|Nothing/));
		await settle();
	}

	test('one link per app with a search box, below the library results (even when those are empty)', async () => {
		const { container } = render(SearchPage);
		expect(container.querySelector('[data-search-on]')).toBeNull();
		await typeTerm(container, 'blue moon');
		const section = container.querySelector('[data-search-on]')!;
		expect(section.querySelector('h2')!.textContent).toBe('Search on…');
		const links = [...section.querySelectorAll<HTMLAnchorElement>('a')];
		expect(links.map((a) => a.getAttribute('data-search-on-app'))).toEqual(['examplemusic', 'examplevideo', 'exampleradio']);
		expect(links[0]!.textContent).toContain('Search Example Music for “blue moon”');
		expect(links[0]!.getAttribute('href')).toBe('#/search-on/examplemusic/blue%20moon');
		// Names are text, never markup.
		expect(links[1]!.textContent).toContain(HOSTILE);
		expect(section.querySelector('img[src="x"]')).toBeNull();
		// Discovery read each app's top menu, with the player, through its own menu command.
		const tops = fake.calls.filter((c) => c.cmd[1] === 'items');
		expect(tops.every((c) => c.player === P && c.cmd[4] === 'menu:1' && c.cmd.length === 5)).toBe(true);
		// No search text was sent to any app while discovering.
		expect(fake.calls.some((c) => c.cmd.some((w) => String(w).includes('blue')) && c.cmd[0] !== 'search' && c.cmd[0] !== 'playlists')).toBe(false);
	});

	test('typing more doesn’t ask again; choosing a row keeps the search as a recent one', async () => {
		const { container } = render(SearchPage);
		await typeTerm(container, 'blue');
		const asked = fake.calls.length;
		await typeTerm(container, 'blue moon');
		expect(fake.calls.filter((c, i) => i >= asked && (c.cmd[0] === 'apps' || c.cmd[1] === 'items'))).toEqual([]);
		const link = container.querySelector<HTMLAnchorElement>('[data-search-on-app="examplemusic"]')!;
		link.addEventListener('click', (e) => e.preventDefault());
		await fireEvent.click(link);
		expect(loadRecents('ann')).toContain('blue moon');
	});

	test('nothing while the box is empty, nor without a player', async () => {
		fake.app.playerId = '';
		const { container } = render(SearchPage);
		await typeTerm(container, 'blue moon');
		expect(container.querySelector('[data-search-on]')).toBeNull();
		expect(fake.calls.some((c) => c.cmd[1] === 'items')).toBe(false);
	});

	test('apps the gateway leaves out for this account are never offered', async () => {
		apps = [{ cmd: 'examplemusic', name: 'Example Music' }];
		const { container } = render(SearchPage);
		await typeTerm(container, 'blue moon');
		const links = [...container.querySelectorAll('[data-search-on-app]')].map((a) => a.getAttribute('data-search-on-app'));
		expect(links).toEqual(['examplemusic', 'exampleradio']);
	});
});

describe('the results page', () => {
	beforeEach(() => {
		fake.calls.length = 0;
		fake.app.playerId = P;
		pluginSearch.reset();
		fake.handler.rpc = async (_p, c) => {
			if (c[0] === 'apps') return { count: 1, appss_loop: [{ cmd: 'examplemusic', name: 'Example Music' }] };
			if (c[0] === 'radios') return { count: 0, radioss_loop: [] };
			if (c[0] === 'examplemusic' && c.some((w) => String(w).startsWith('search:'))) {
				return { count: 1, item_loop: [{ text: 'Found track', type: 'audio', actions: { play: { cmd: ['examplemusic', 'playlist', 'play'], params: { item_id: '0.1' } }, add: { cmd: ['examplemusic', 'playlist', 'add'], params: { item_id: '0.1' } } } }] };
			}
			if (c[0] === 'examplemusic') return topMenu('examplemusic');
			return {};
		};
	});

	const searches = (): Cmd[] => fake.calls.filter((c) => c.cmd.some((w) => String(w).startsWith('search:'))).map((c) => c.cmd);

	test('opens the app’s results in the menu browser, where play and add work', async () => {
		params.app = 'examplemusic';
		params.q = 'blue moon';
		const { container, findByText } = render(ResultsPage);
		await findByText('Found track');
		expect(container.querySelector('h2')!.textContent).toBe('Example Music: “blue moon”');
		expect(searches()).toEqual([['examplemusic', 'items', 0, 100, 'item_id:0', 'search:blue moon', 'menu:examplemusic']]);
		await choose('More options for Found track', 'Add to queue', container);
		await settle();
		expect(fake.calls.at(-1)!.cmd).toEqual(['examplemusic', 'playlist', 'add', 'item_id:0.1']);
		expect(fake.calls.at(-1)!.player).toBe(P);
	});

	test('a hostile query goes only as the search value', async () => {
		params.app = 'examplemusic';
		params.q = 'x item_id:9 url:file:///etc/passwd\nmenu:1';
		const { findByText } = render(ResultsPage);
		await findByText('Found track');
		const [c] = searches();
		expect(c).toEqual(['examplemusic', 'items', 0, 100, 'item_id:0', 'search:x item_id:9 url:file:///etc/passwd menu:1', 'menu:examplemusic']);
	});

	test('an app that isn’t listed, has no search box or no text sends no search', async () => {
		for (const [app, q, says] of [
			['notlisted', 'abc', 'any more'],
			['../etc', 'abc', 'any more'],
			['examplemusic', '   ', 'Type something'],
		] as const) {
			params.app = app;
			params.q = q;
			const { findByText, unmount } = render(ResultsPage);
			await findByText(new RegExp(says));
			unmount();
		}
		expect(searches()).toEqual([]);
	});
});
