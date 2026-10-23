// The Search page (Wave 2 §3.3): recent searches (this device only, removable, Clear all),
// grouped results with a Top result, type chips that page one type alone, and playing from
// results (row, play button, menu, keys). Invented names only.
import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { loadRecents, saveRecents } from '#lib/search/recents';

vi.mock('#lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const fake = await import('./helpers/fake-app.svelte');
const { search } = await import('#lib/state/search');
const SearchPage = (await import('../src/routes/search/+page.svelte')).default;

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

const ANSWER = {
	contributors_count: 7,
	contributors_loop: Array.from({ length: 7 }, (_, i) => ({ contributor: i ? `Placeholder Band ${i}` : 'Placeholder', contributor_id: 10 + i })),
	albums_count: 1,
	albums_loop: [{ album: 'Placeholder Album', album_id: 70 }],
	tracks_count: 1,
	tracks_loop: [{ track: HOSTILE, track_id: 900 }],
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
	fake.session.current = { user: { username: 'ann', role: 'listener' }, device: { id: 1, name: 'Phone' } };
	localStorage.clear();
	search.reset();
	fake.handler.rpc = async (_p, c) => {
		if (c[0] === 'search') return ANSWER;
		if (c[0] === 'playlists') return { count: 1, playlists_loop: [{ id: 5, playlist: 'Placeholder mix' }] };
		if (c[0] === 'artists') {
			const from = Number(c[1]);
			return { count: 7, artists_loop: ANSWER.contributors_loop.slice(from).map((a) => ({ id: a.contributor_id, artist: a.contributor })) };
		}
		return {};
	};
});

async function typeTerm(container: HTMLElement, term: string) {
	const box = container.querySelector<HTMLInputElement>('[data-search-input]')!;
	await fireEvent.input(box, { target: { value: term } });
	await waitFor(() => expect(container.querySelector('[role="status"]')!.textContent).toMatch(/result/));
	return box;
}

test('one grouped search: Top result, then each type with "See all"', async () => {
	const { container } = render(SearchPage);
	await typeTerm(container, 'placeholder');
	expect(sent('search')[0]).toEqual(['search', 0, 20, 'term:placeholder', 'extended:1']);
	expect(sent('playlists')[0]).toEqual(['playlists', 0, 20, 'search:placeholder']);
	// The exact artist match is the Top result.
	expect(container.querySelector('.top [data-hit]')?.getAttribute('data-hit')).toBe('artist');
	expect(container.querySelector('.top')?.textContent).toContain('Placeholder');
	expect([...container.querySelectorAll('[data-group]')].map((g) => g.getAttribute('data-group'))).toEqual(['artists', 'albums', 'tracks', 'playlists']);
	// Five artists shown, with a way to all seven.
	expect(container.querySelectorAll('[data-group="artists"] li').length).toBe(5);
	expect(container.querySelector('[data-see-all="artists"]')?.textContent).toBe('See all 7 artists');
	// Hostile names are text.
	expect(container.textContent).toContain(HOSTILE);
	expect(container.querySelectorAll('img, script').length).toBe(0);
	expect(container.querySelector('[role="status"]')!.textContent).toBe('10 results for “placeholder”.');
});

test('a type chip pages through that type alone', async () => {
	const { container } = render(SearchPage);
	await typeTerm(container, 'placeholder');
	await fireEvent.click(container.querySelector('[data-see-all="artists"]')!);
	await waitFor(() => expect(container.querySelectorAll('[data-type-list="artists"] li').length).toBe(7));
	expect(sent('artists')[0]).toEqual(['artists', 0, 40, 'search:placeholder']);
	expect(container.querySelector('[role="tab"][aria-selected="true"]')?.textContent?.trim()).toBe('Artists');
	// Arrow keys move between type chips.
	const tab = container.querySelector<HTMLButtonElement>('[role="tab"][aria-selected="true"]')!;
	await fireEvent.keyDown(tab, { key: 'ArrowRight' });
	await waitFor(() => expect(sent('albums').length).toBe(1));
	expect(sent('albums')[0]).toEqual(['albums', 0, 40, 'tags:aljyS', 'sort:album', 'search:placeholder']);
	await fireEvent.keyDown(container.querySelector('[role="tab"][aria-selected="true"]')!, { key: 'ArrowRight' });
	await waitFor(() => expect(sent('titles').length).toBe(1));
	expect(sent('titles')[0]).toEqual(['titles', 0, 40, 'tags:adtlcJKj', 'search:placeholder']);
});

test('playing from results: a track plays from its row; albums have a play button and a menu', async () => {
	const { container, getAllByRole } = render(SearchPage);
	await typeTerm(container, 'placeholder');
	await fireEvent.click(container.querySelector('[data-group="tracks"] [data-hit-play]')!);
	expect(sent('playlistcontrol').at(-1)).toEqual(['playlistcontrol', 'cmd:load', 'track_id:900']);
	await fireEvent.click(container.querySelector('[data-group="albums"] [data-hit-play]')!);
	expect(sent('playlistcontrol').at(-1)).toEqual(['playlistcontrol', 'cmd:load', 'album_id:70']);
	// A long press (or right-click) opens the row's menu: Play next and Add to queue.
	await fireEvent.contextMenu(container.querySelector('[data-group="albums"] [data-hit]')!);
	await settle();
	const items = getAllByRole('menuitem');
	expect(items.map((i) => i.textContent?.replace(/[NA]$/, '').trim())).toEqual(['Play next', 'Add to queue']);
	await fireEvent.click(items[0]!);
	expect(sent('playlistcontrol').at(-1)).toEqual(['playlistcontrol', 'cmd:insert', 'album_id:70']);
	// Keys on a focused row.
	await fireEvent.keyDown(container.querySelector('[data-group="tracks"] [data-hit-play]')!, { key: 'a' });
	expect(sent('playlistcontrol').at(-1)).toEqual(['playlistcontrol', 'cmd:add', 'track_id:900']);
	// Playing from a result remembers the search.
	expect(loadRecents('ann')).toEqual(['placeholder']);
});

test('recent searches: shown with an empty box, reusable, removable and cleared; never sent anywhere', async () => {
	saveRecents('ann', ['zebra stripes', 'placeholder']);
	const { container, getByRole } = render(SearchPage);
	expect(container.querySelector('[data-recents]')).not.toBeNull();
	expect([...container.querySelectorAll('[data-recents] .term')].map((b) => b.textContent?.trim())).toEqual(['zebra stripes', 'placeholder']);
	await fireEvent.click(getByRole('button', { name: 'Remove “zebra stripes” from recent searches' }));
	expect(loadRecents('ann')).toEqual(['placeholder']);
	await fireEvent.click(container.querySelector('[data-recents] .term')!);
	await waitFor(() => expect(sent('search').length).toBe(1));
	expect(container.querySelector<HTMLInputElement>('[data-search-input]')!.value).toBe('placeholder');
	// Recents hide while there's a term.
	expect(container.querySelector('[data-recents]')).toBeNull();
	search.set('');
	await settle();
	await fireEvent.click(container.querySelector('[data-recents-clear]')!);
	expect(container.querySelector('[data-recents]')).toBeNull();
	expect(loadRecents('ann')).toEqual([]);
	// No request ever carried the recents.
	for (const c of fake.calls) expect(JSON.stringify(c.cmd)).not.toContain('zebra stripes');
});

test('Enter remembers the term; another user doesn’t see it', async () => {
	const { container } = render(SearchPage);
	const box = await typeTerm(container, 'placeholder');
	await fireEvent.submit(box.closest('form')!);
	expect(loadRecents('ann')).toEqual(['placeholder']);
	expect(loadRecents('bob')).toEqual([]);
});

test('a failing playlist search still shows the rest', async () => {
	fake.handler.rpc = async (_p, c) => {
		if (c[0] === 'playlists') throw new Error('refused');
		return c[0] === 'search' ? ANSWER : {};
	};
	const { container } = render(SearchPage);
	await typeTerm(container, 'placeholder');
	expect(container.querySelector('[data-group="artists"]')).not.toBeNull();
	expect(container.querySelector('[data-group="playlists"]')).toBeNull();
});
