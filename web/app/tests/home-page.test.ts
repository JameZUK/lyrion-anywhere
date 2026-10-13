// Home: shelves in the user's order, and edit mode (move, hide, add, save, revert).
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import type { HomeLayout, Station } from '$lib/api/me';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const saves: HomeLayout[] = [];
const fakeMe = {
	layout: { version: 1, shelves: [{ type: 'recent' }, { type: 'stations' }, { type: 'new' }], default: true } as HomeLayout,
	stations: [{ id: 4, name: 'Fresh', kind: 'fresh', spec: { kind: 'fresh' }, broken: null }] as Station[],
	failSave: false,
	home: vi.fn(async () => structuredClone(fakeMe.layout)),
	stations_: null,
	menus: vi.fn(async () => []),
	shelf: vi.fn(async () => [7]),
	saveHome: vi.fn(async (l: HomeLayout) => {
		if (fakeMe.failSave) throw new Error('gateway said no');
		saves.push(structuredClone(l));
		return { ...l, default: false };
	}),
	resetHome: vi.fn(),
	deleteStation: vi.fn(),
	next: vi.fn(),
	openMenu: vi.fn(),
};
vi.mock('$lib/state/me', () => ({ me: { ...fakeMe, stations: async () => fakeMe.stations } }));
const fake = await import('./helpers/fake-app.svelte');
const { default: Home } = await import('../src/routes/home/+page.svelte');

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
	saves.length = 0;
	fakeMe.failSave = false;
	fake.calls.length = 0;
	fake.app.playerId = '02:00:00:00:00:01';
	fake.handler.rpc = async (_p, cmd) => {
		if (cmd[0] === 'albums') return { albums_loop: [{ id: 7, album: 'Blue Album', artist: 'Someone' }] };
		return {};
	};
});

const headings = () => screen.getAllByRole('heading', { level: 2 }).map((h) => h.textContent);

test('shelves show in the saved order, with albums by id', async () => {
	render(Home);
	await waitFor(() => expect(headings()).toEqual(['Recently played', 'Stations', 'New music']));
	await waitFor(() => expect(screen.getAllByText('Blue Album').length).toBeGreaterThan(0));
	// The recent shelf asked for its ids, then those albums by id.
	expect(fake.calls.some((c) => c.cmd[0] === 'albums' && String(c.cmd.at(-1)) === 'album_id:7')).toBe(true);
	expect(screen.getByRole('button', { name: 'Play the station Fresh' })).toBeTruthy();
	expect(screen.getByRole('link', { name: /Browse the library/ }).getAttribute('href')).toBe('#/library');
});

test('edit mode: move, hide and add with accessible controls, then save', async () => {
	render(Home);
	await fireEvent.click(await screen.findByRole('button', { name: 'Edit Home' }));
	expect(screen.getByRole('button', { name: 'Move Recently played up' }).hasAttribute('disabled')).toBe(true);
	await fireEvent.click(screen.getByRole('button', { name: 'Move Recently played down' }));
	await fireEvent.click(screen.getByRole('button', { name: 'Hide New music' }));
	const pick = screen.getByLabelText('Add a shelf') as HTMLSelectElement;
	await fireEvent.change(pick, { target: { value: 'most' } });
	await fireEvent.click(screen.getByRole('button', { name: 'Add' }));
	const list = screen.getByRole('list', { name: 'Home shelves' });
	expect(list.textContent).toContain('Most played');
	await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
	await waitFor(() => expect(saves).toHaveLength(1));
	expect(saves[0]).toEqual({
		version: 1,
		shelves: [{ type: 'stations' }, { type: 'recent' }, { type: 'new', hidden: true }, { type: 'most' }],
	});
	// Back to the shelves, without the hidden one.
	await waitFor(() => expect(headings()).toEqual(['Stations', 'Recently played', 'Most played']));
});

test('a failed save puts the layout back and says why', async () => {
	fakeMe.failSave = true;
	render(Home);
	await fireEvent.click(await screen.findByRole('button', { name: 'Edit Home' }));
	await fireEvent.click(screen.getByRole('button', { name: 'Remove Recently played' }));
	await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
	await waitFor(() => expect(screen.getByRole('status').textContent).toContain('gateway said no'));
	await fireEvent.click(screen.getByRole('button', { name: 'Edit Home' }));
	expect(screen.getByRole('list', { name: 'Home shelves' }).textContent).toContain('Recently played');
});

test('favourites and playlists shelves list what LMS has', async () => {
	const keep = fakeMe.layout;
	fakeMe.layout = { version: 1, shelves: [{ type: 'favourites' }, { type: 'playlists' }] };
	fake.handler.rpc = async (_p, cmd) => {
		if (cmd[0] === 'favorites') return { loop_loop: [{ id: 'f.1', name: 'Morning radio', isaudio: 1 }, { id: 'f.2', name: 'A folder', hasitems: 1 }] };
		if (cmd[0] === 'playlists') return { playlists_loop: [{ id: 12, playlist: 'Road trip' }] };
		return {};
	};
	render(Home);
	await waitFor(() => expect(headings()).toEqual(['Favourites', 'Playlists']));
	expect(await screen.findByRole('button', { name: 'Play Morning radio' })).toBeTruthy();
	expect(screen.getByRole('link', { name: 'A folder' }).getAttribute('href')).toBe('#/favourites');
	expect(screen.getByRole('link', { name: 'Road trip' }).getAttribute('href')).toBe('#/playlist/12');
	fakeMe.layout = keep;
});

test('a shelf of a station needs one picked, and is never added twice', async () => {
	render(Home);
	await fireEvent.click(await screen.findByRole('button', { name: 'Edit Home' }));
	await fireEvent.change(screen.getByLabelText('Add a shelf'), { target: { value: 'station' } });
	await fireEvent.click(screen.getByRole('button', { name: 'Add' }));
	expect(screen.getByRole('status').textContent).toContain('Choose which one');
	await fireEvent.change(screen.getByLabelText('Which'), { target: { value: '4' } });
	await fireEvent.click(screen.getByRole('button', { name: 'Add' }));
	expect(screen.getByRole('list', { name: 'Home shelves' }).textContent).toContain('Station: Fresh');
	await fireEvent.change(screen.getByLabelText('Add a shelf'), { target: { value: 'station' } });
	await fireEvent.change(screen.getByLabelText('Which'), { target: { value: '4' } });
	await fireEvent.click(screen.getByRole('button', { name: 'Add' }));
	expect(screen.getByRole('status').textContent).toContain('already on Home');
});
