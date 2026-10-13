// Smart filters in the web app (wave2-design.md §5.6): the rule builder (its checks mirror the
// contract's limits), the Library chip with list/create/edit/delete (deleting asks first),
// the filter page, the `filter` Home shelf, and play/shuffle through the album play path.
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import type { FilterSpec, SmartFilter } from '$lib/api/filters';
import * as L from '$lib/home/layout';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const params: Record<string, string> = { id: '3' };
vi.mock('$app/state', () => ({ page: { params, url: new URL('http://app.example/#/') } }));

const store = {
	filters: [] as SmartFilter[],
	nextId: 10,
	albums: [] as number[],
	created: [] as FilterSpec[],
	updated: [] as [number, FilterSpec][],
	deleted: [] as number[],
	asked: [] as [number, number, number][],
};
vi.mock('$lib/state/me', () => ({
	me: {
		filters: async () => structuredClone(store.filters),
		createFilter: async (spec: FilterSpec) => {
			store.created.push(spec);
			const id = store.nextId++;
			store.filters.push({ id, ...spec });
			return id;
		},
		updateFilter: async (id: number, spec: FilterSpec) => {
			store.updated.push([id, spec]);
			store.filters = store.filters.map((f) => (f.id === id ? { id, ...spec } : f));
			return {};
		},
		deleteFilter: async (id: number) => {
			store.deleted.push(id);
			store.filters = store.filters.filter((f) => f.id !== id);
			return {};
		},
		filterAlbums: async (id: number, from: number, n: number) => {
			store.asked.push([id, from, n]);
			return { albums: store.albums.slice(from, from + n), count: store.albums.length };
		},
		home: async () => ({ version: 1, shelves: [] }),
		stations: async () => [],
		menus: async () => [],
		shelf: async () => [],
	},
}));

const fake = await import('./helpers/fake-app.svelte');
const { default: FilterEditor } = await import('$lib/components/FilterEditor.svelte');
const { default: FiltersView } = await import('$lib/components/FiltersView.svelte');
const { default: HomeShelf } = await import('$lib/components/HomeShelf.svelte');
const { default: LibraryPage } = await import('../src/routes/library/+page.svelte');
const { default: FilterPage } = await import('../src/routes/filter/[id]/+page.svelte');
const { LIBRARY_KEY } = await import('$lib/library/prefs');

const PLAYER = '02:00:00:00:00:01';
const GENRES = Array.from({ length: 25 }, (_, i) => ({ id: i + 1, genre: `Genre ${i + 1}` }));
const jazz: SmartFilter = { id: 3, name: 'Late jazz', rules: [{ field: 'genre', in: [2] }, { field: 'decade', is: 1960 }], sort: 'yearalbum', limit: 100 };

beforeAll(() => {
	globalThis.IntersectionObserver = class {
		observe() {}
		disconnect() {}
		unobserve() {}
	} as unknown as typeof IntersectionObserver;
});

beforeEach(() => {
	store.filters = [structuredClone(jazz)];
	store.albums = [];
	store.created.length = 0;
	store.updated.length = 0;
	store.deleted.length = 0;
	store.asked.length = 0;
	fake.calls.length = 0;
	fake.app.playerId = PLAYER;
	fake.app.players = [{ playerid: PLAYER, name: 'Test player' }];
	fake.handler.rpc = async (_p, cmd) => {
		if (cmd[0] === 'genres') return { genres_loop: GENRES };
		if (cmd[0] === 'artists') return { artists_loop: [{ id: 41, artist: 'Example Quartet' }, { id: 0, artist: 'No id' }] };
		if (cmd[0] === 'albums') {
			const ids = String(cmd.find((x) => String(x).startsWith('album_id:')) ?? '').slice(9).split(',').filter(Boolean).map(Number);
			return { albums_loop: ids.map((id) => ({ id, album: `Album ${id}`, artist: 'Example Band' })) };
		}
		return {};
	};
});

const sentPlay = () => fake.calls.filter((c) => c.cmd[0] === 'playlistcontrol' || c.cmd[0] === 'playlist').map((c) => c.cmd);

describe('the rule builder', () => {
	function editor(initial: SmartFilter | null = null) {
		const onsave = vi.fn(async (_spec: FilterSpec) => '');
		const oncancel = vi.fn();
		render(FilterEditor, { initial, onsave, oncancel });
		return { onsave, oncancel, dialog: screen.getByRole('dialog') };
	}
	const save = () => fireEvent.click(screen.getByRole('button', { name: 'Save' }));
	const errorText = () => document.querySelector('[data-filter-error]')!.textContent;
	async function addRule(label: string) {
		await fireEvent.change(screen.getByLabelText('Add a rule'), { target: { value: label } });
		await fireEvent.click(screen.getByRole('button', { name: 'Add' }));
	}

	test('needs a name and at least one rule; nothing is saved until it is valid', async () => {
		const { onsave } = editor();
		await save();
		expect(errorText()).toMatch(/name/i);
		await fireEvent.input(screen.getByLabelText('Name'), { target: { value: '🎵' } });
		await save();
		expect(errorText()).toMatch(/name/i);
		await fireEvent.input(screen.getByLabelText('Name'), { target: { value: 'Quiet evenings' } });
		await save();
		expect(errorText()).toMatch(/at least one rule/i);
		expect(onsave).not.toHaveBeenCalled();
		// The message is announced.
		expect(screen.getByRole('alert').textContent).toBe(errorText());
	});

	test('genres: from the server, at most 20, then saved by id', async () => {
		const { onsave } = editor();
		await fireEvent.input(screen.getByLabelText('Name'), { target: { value: 'Lots of genres' } });
		await addRule('genre');
		await waitFor(() => expect(screen.getAllByRole('checkbox')).toHaveLength(25));
		await save();
		expect(errorText()).toMatch(/at least one genre/i);
		for (const g of GENRES.slice(0, 20)) await fireEvent.click(screen.getByRole('checkbox', { name: g.genre }));
		expect(screen.getByText('20 of up to 20 chosen')).toBeTruthy();
		// The rest can't be ticked.
		expect((screen.getByRole('checkbox', { name: 'Genre 21' }) as HTMLInputElement).disabled).toBe(true);
		await save();
		expect(onsave).toHaveBeenCalledTimes(1);
		const spec = onsave.mock.calls[0]![0];
		expect(spec.rules).toEqual([{ field: 'genre', in: GENRES.slice(0, 20).map((g) => g.id) }]);
		expect(spec).toMatchObject({ name: 'Lots of genres', sort: 'album', limit: 100 });
	});

	test('a decade must be chosen; each kind of rule is offered once', async () => {
		const { onsave } = editor();
		await fireEvent.input(screen.getByLabelText('Name'), { target: { value: 'Seventies' } });
		await addRule('decade');
		const options = [...(screen.getByLabelText('Add a rule') as HTMLSelectElement).options].map((o) => o.value);
		expect(options).not.toContain('decade');
		await save();
		expect(errorText()).toMatch(/decade/i);
		await fireEvent.change(screen.getByLabelText('Decade is'), { target: { value: '1970' } });
		await save();
		expect(onsave.mock.calls[0]![0].rules).toEqual([{ field: 'decade', is: 1970 }]);
	});

	test('an artist is picked from the server’s list, optionally in a role', async () => {
		const { onsave } = editor();
		await fireEvent.input(screen.getByLabelText('Name'), { target: { value: 'By one composer' } });
		await addRule('artist');
		await save();
		expect(errorText()).toMatch(/artist/i);
		await fireEvent.input(screen.getByLabelText('Find an artist'), { target: { value: 'Example' } });
		await fireEvent.click(await screen.findByRole('button', { name: 'Example Quartet' }, { timeout: 2000 }));
		// Only ids from the server: the one without an id isn't offered.
		expect(screen.queryByRole('button', { name: 'No id' })).toBeNull();
		await fireEvent.change(screen.getByLabelText('In the role'), { target: { value: 'COMPOSER' } });
		await save();
		expect(onsave.mock.calls[0]![0].rules).toEqual([{ field: 'artist', is: 41, role: 'COMPOSER' }]);
	});

	test('compilation, order and the limit (1–500)', async () => {
		const { onsave } = editor();
		await fireEvent.input(screen.getByLabelText('Name'), { target: { value: 'No compilations' } });
		await addRule('compilation');
		await fireEvent.click(screen.getByRole('radio', { name: 'No compilations' }));
		await fireEvent.change(screen.getByLabelText('Order'), { target: { value: 'random' } });
		await fireEvent.input(screen.getByLabelText('Up to (albums)'), { target: { value: '501' } });
		await save();
		expect(errorText()).toMatch(/1 to 500/);
		await fireEvent.input(screen.getByLabelText('Up to (albums)'), { target: { value: '0' } });
		await save();
		expect(errorText()).toMatch(/1 to 500/);
		await fireEvent.input(screen.getByLabelText('Up to (albums)'), { target: { value: '500' } });
		await save();
		expect(onsave.mock.calls[0]![0]).toEqual({ name: 'No compilations', rules: [{ field: 'compilation', is: false }], sort: 'random', limit: 500 });
	});

	test('editing starts from the saved filter; a rule can be removed', async () => {
		const { onsave } = editor(jazz);
		expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Late jazz');
		expect((screen.getByLabelText('Decade is') as HTMLSelectElement).value).toBe('1960');
		await waitFor(() => expect((screen.getByRole('checkbox', { name: 'Genre 2' }) as HTMLInputElement).checked).toBe(true));
		await fireEvent.click(screen.getByRole('button', { name: 'Remove the genre rule' }));
		await save();
		expect(onsave.mock.calls[0]![0]).toEqual({ name: 'Late jazz', rules: [{ field: 'decade', is: 1960 }], sort: 'yearalbum', limit: 100 });
	});

	test('a refusal from the gateway is shown and the editor stays open', async () => {
		const onsave = vi.fn(async () => 'Couldn’t save the filter: too many filters');
		render(FilterEditor, { initial: jazz, onsave, oncancel: () => {} });
		await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
		await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/too many/));
	});
});

describe('the Filters chip in Library', () => {
	test('Library has a Filters chip showing the list', async () => {
		localStorage.removeItem(LIBRARY_KEY);
		render(LibraryPage);
		const tab = screen.getByRole('tab', { name: 'Filters' });
		await fireEvent.click(tab);
		expect(tab.getAttribute('aria-selected')).toBe('true');
		expect(await screen.findByRole('link', { name: /Late jazz/ })).toBeTruthy();
		expect(JSON.parse(localStorage.getItem(LIBRARY_KEY)!).chip).toBe('filters');
	});

	test('create: the builder saves a new filter, which is then listed', async () => {
		render(FiltersView);
		await fireEvent.click(await screen.findByRole('button', { name: 'New filter' }));
		await fireEvent.input(screen.getByLabelText('Name'), { target: { value: 'Compilations' } });
		await fireEvent.change(screen.getByLabelText('Add a rule'), { target: { value: 'compilation' } });
		await fireEvent.click(screen.getByRole('button', { name: 'Add' }));
		await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
		await waitFor(() => expect(store.created).toHaveLength(1));
		expect(store.created[0]).toEqual({ name: 'Compilations', rules: [{ field: 'compilation', is: true }], sort: 'album', limit: 100 });
		expect(await screen.findByRole('link', { name: /Compilations/ })).toBeTruthy();
		expect(screen.getByRole('status').textContent).toMatch(/Added/);
		expect(screen.queryByRole('dialog')).toBeNull();
	});

	test('edit replaces the filter', async () => {
		render(FiltersView);
		await fireEvent.click(await screen.findByRole('button', { name: 'Edit Late jazz' }));
		await fireEvent.input(screen.getByLabelText('Name'), { target: { value: 'Early jazz' } });
		await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
		await waitFor(() => expect(store.updated).toHaveLength(1));
		expect(store.updated[0]![0]).toBe(3);
		expect(store.updated[0]![1].name).toBe('Early jazz');
		expect(await screen.findByRole('link', { name: /Early jazz/ })).toBeTruthy();
	});

	test('delete asks first; Cancel keeps it', async () => {
		render(FiltersView);
		await fireEvent.click(await screen.findByRole('button', { name: 'Delete Late jazz' }));
		const confirm = screen.getByRole('alertdialog');
		expect(within(confirm).getByText('Delete “Late jazz”?')).toBeTruthy();
		await fireEvent.click(within(confirm).getByRole('button', { name: 'Cancel' }));
		expect(store.deleted).toEqual([]);
		await fireEvent.click(screen.getByRole('button', { name: 'Delete Late jazz' }));
		await fireEvent.click(within(screen.getByRole('alertdialog')).getByRole('button', { name: 'Delete' }));
		await waitFor(() => expect(store.deleted).toEqual([3]));
		await waitFor(() => expect(screen.queryByRole('link', { name: /Late jazz/ })).toBeNull());
	});

	test('play loads the first albums through the album play path; shuffle turns shuffle on first', async () => {
		store.albums = Array.from({ length: 70 }, (_, i) => 100 + i);
		render(FiltersView);
		await fireEvent.click(await screen.findByRole('button', { name: 'Play Late jazz' }));
		await waitFor(() => expect(sentPlay()).toHaveLength(50));
		expect(store.asked[0]).toEqual([3, 0, 50]);
		expect(sentPlay()[0]).toEqual(['playlistcontrol', 'cmd:load', 'album_id:100']);
		expect(sentPlay()[49]).toEqual(['playlistcontrol', 'cmd:add', 'album_id:149']);
		expect(fake.calls.every((c) => c.player === PLAYER)).toBe(true);
		fake.calls.length = 0;
		await fireEvent.click(screen.getByRole('button', { name: 'Shuffle Late jazz' }));
		await waitFor(() => expect(sentPlay()).toHaveLength(51));
		expect(sentPlay()[0]).toEqual(['playlist', 'shuffle', 1]);
	});

	test('without a player, nothing is sent and it says why', async () => {
		fake.app.playerId = '';
		store.albums = [1];
		render(FiltersView);
		await fireEvent.click(await screen.findByRole('button', { name: 'Play Late jazz' }));
		await waitFor(() => expect(screen.getByRole('status').textContent).toMatch(/Choose a player/));
		expect(sentPlay()).toEqual([]);
	});

	test('every control has an accessible name', async () => {
		render(FiltersView);
		await screen.findByRole('link', { name: /Late jazz/ });
		for (const b of screen.getAllByRole('button')) expect(b.getAttribute('aria-label') || b.textContent?.trim()).toBeTruthy();
	});
});

describe('the filter page', () => {
	test('shows its albums, pages through them and plays', async () => {
		store.albums = [5, 6, 7];
		render(FilterPage);
		expect(await screen.findByRole('heading', { level: 1, name: 'Late jazz' })).toBeTruthy();
		await waitFor(() => expect(screen.getByText('Album 6')).toBeTruthy());
		expect(screen.getByText('3 albums')).toBeTruthy();
		await fireEvent.click(screen.getByRole('button', { name: 'Shuffle' }));
		await waitFor(() => expect(sentPlay()).toEqual([
			['playlist', 'shuffle', 1],
			['playlistcontrol', 'cmd:load', 'album_id:5'],
			['playlistcontrol', 'cmd:add', 'album_id:6'],
			['playlistcontrol', 'cmd:add', 'album_id:7'],
		]));
	});

	test('a filter that isn’t the user’s says so', async () => {
		params.id = '99';
		try {
			render(FilterPage);
			expect(await screen.findByText(/deleted, or isn’t yours/)).toBeTruthy();
		} finally {
			params.id = '3';
		}
	});

	test('an id that isn’t one asks for nothing', async () => {
		params.id = '3;drop';
		try {
			render(FilterPage);
			expect(await screen.findByText('Not a filter.')).toBeTruthy();
			expect(store.asked).toEqual([]);
		} finally {
			params.id = '3';
		}
	});
});

describe('the filter Home shelf', () => {
	test('a `filter` shelf needs an id, and can be added once per filter', () => {
		expect(L.shelfProblem({ type: 'filter', id: 3 })).toBeNull();
		expect(L.shelfProblem({ type: 'filter' })).not.toBeNull();
		expect(L.shelfProblem({ type: 'filter', id: 0 })).not.toBeNull();
		const l = L.add(L.defaultLayout(), { type: 'filter', id: 3 });
		expect(() => L.add(l, { type: 'filter', id: 3 })).toThrow(/already/);
		expect(L.addable(l)).toContain('filter');
		expect(L.SHELF_LABELS.filter).toBe('Smart filter');
	});

	test('shows the filter’s albums under its name, with Play and Shuffle', async () => {
		store.albums = [8, 9];
		render(HomeShelf, { shelf: { type: 'filter', id: 3 }, stations: [], menus: [], filters: [jazz] });
		expect(await screen.findByRole('heading', { level: 2, name: 'Late jazz' })).toBeTruthy();
		await waitFor(() => expect(screen.getByText('Album 9')).toBeTruthy());
		expect(store.asked[0]).toEqual([3, 0, 20]);
		await fireEvent.click(screen.getByRole('button', { name: 'Play Late jazz' }));
		await waitFor(() => expect(sentPlay()).toEqual([
			['playlistcontrol', 'cmd:load', 'album_id:8'],
			['playlistcontrol', 'cmd:add', 'album_id:9'],
		]));
	});

	test('an empty filter says so', async () => {
		render(HomeShelf, { shelf: { type: 'filter', id: 3 }, stations: [], menus: [], filters: [jazz] });
		expect(await screen.findByText('No albums match this filter.')).toBeTruthy();
		expect(screen.queryByRole('button', { name: 'Play Late jazz' })).toBeNull();
	});
});
