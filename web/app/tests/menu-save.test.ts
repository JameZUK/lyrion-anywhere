// "Save as station" and "Pin to Home" in menus: what the gateway is sent is exactly the
// menu requests that led to the item and the item's own offered actions; never typed text.
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { ApiError } from '$lib/api/http';

const P = '02:4c:41:00:00:01';
const created: { name: string; spec: unknown; player?: string }[] = [];
const pinned: { name: string; steps: unknown; player: string }[] = [];
const saved: unknown[] = [];
let refuse = false;
let menus: Record<string, unknown> = {};

vi.mock('$lib/state/app.svelte', () => ({
	app: { playerId: P, soon() {}, queueVersion: 0 },
	api: {
		async rpc(_player: string, cmd: unknown[]) {
			return menus[String(cmd[0])] ?? { item_loop: [] };
		},
		art: async () => 'blob:x',
	},
}));
vi.mock('$lib/state/me', () => ({
	me: {
		async createStation(name: string, spec: unknown, player?: string) {
			if (refuse) throw new ApiError(422, 'this menu item is no longer offered');
			created.push({ name, spec, player });
			return { id: 1, name };
		},
		async createMenu(name: string, steps: unknown, player: string) {
			pinned.push({ name, steps, player });
			return { id: 8, name, steps, broken: null };
		},
		async home() {
			return { version: 1, shelves: [{ type: 'recent' }] };
		},
		async saveHome(l: unknown) {
			saved.push(l);
			return l;
		},
	},
}));

const MenuView = (await import('$lib/components/MenuView.svelte')).default;

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
	created.length = 0;
	pinned.length = 0;
	saved.length = 0;
	refuse = false;
	menus = {
		trackinfo: {
			count: 4,
			base: { actions: { add: { cmd: ['audiomuse', 'mix', 'add'], params: { menu: 'm' }, itemsParams: 'params' } } },
			item_loop: [
				{ text: 'Mix like this', nextWindow: 'nowPlaying', params: { seed: '9' }, actions: { go: { cmd: ['audiomuse', 'mix', 'play'], params: { seed: '9' } } } },
				{ text: 'Describe a mix', nextWindow: 'refresh', actions: { go: { cmd: ['audiomuse', 'instant'], params: { prompt: '__TAGGEDINPUT__' } } }, input: {} },
				{ text: 'Similar artists', actions: { go: { cmd: ['audiomuse', 'artists'], params: { id: '4' } } } },
				{ text: 'Just words' },
			],
		},
	};
});

const start = (from: number, n: number) => ['trackinfo', 'items', from, n, 'track_id:9', 'menu:1'];
const settle = async () => {
	for (let i = 0; i < 5; i++) {
		await tick();
		await new Promise((r) => setTimeout(r, 0));
	}
};

test('a mix is saved as a reference to what the menu offered', async () => {
	render(MenuView, { start, title: 'Song' });
	await settle();
	await fireEvent.click(screen.getByRole('button', { name: 'Save Mix like this as a station' }));
	await waitFor(() => expect(created).toHaveLength(1));
	expect(created[0]).toEqual({
		name: 'Mix like this',
		player: P,
		spec: {
			kind: 'menu',
			steps: [['trackinfo', 'items', 0, 100, 'track_id:9', 'menu:1']],
			play: ['audiomuse', 'mix', 'play', 'seed:9'],
			add: ['audiomuse', 'mix', 'add', 'menu:m', 'seed:9'],
		},
	});
	expect(screen.getByRole('status').textContent).toContain('Saved “Mix like this” as a station');
});

test('items needing typed text, text and folders are never saved as stations', async () => {
	render(MenuView, { start, title: 'Song' });
	await settle();
	expect(screen.queryByRole('button', { name: /Describe a mix as a station/ })).toBeNull();
	expect(screen.queryByRole('button', { name: /Similar artists as a station/ })).toBeNull();
	expect(screen.queryByRole('button', { name: /Just words/ })).toBeNull();
});

test('a refusal says why', async () => {
	refuse = true;
	render(MenuView, { start, title: 'Song' });
	await settle();
	await fireEvent.click(screen.getByRole('button', { name: 'Save Mix like this as a station' }));
	await waitFor(() => expect(screen.getByRole('status').textContent).toBe('This can’t be saved: this menu item is no longer offered'));
});

test('a folder is pinned to Home with the request that opens it as the last step', async () => {
	render(MenuView, { start, title: 'Song' });
	await settle();
	await fireEvent.click(screen.getByRole('button', { name: 'Pin Similar artists to Home' }));
	await waitFor(() => expect(saved).toHaveLength(1));
	expect(pinned[0]).toEqual({
		name: 'Similar artists',
		player: P,
		steps: [
			['trackinfo', 'items', 0, 100, 'track_id:9', 'menu:1'],
			['audiomuse', 'artists', 0, 100, 'id:4'],
		],
	});
	expect(saved[0]).toEqual({ version: 1, shelves: [{ type: 'recent' }, { type: 'menu', id: 8 }] });
});

test('a menu handed over whole needs its root request to be saved', async () => {
	const initial = menus.trackinfo as object;
	const { unmount } = render(MenuView, { initial, title: 'Group' });
	await settle();
	expect(screen.queryByRole('button', { name: /as a station/ })).toBeNull();
	unmount();
	render(MenuView, { initial, title: 'Group', root: ['menu', 0, 500, 'direct:1'] });
	await settle();
	await fireEvent.click(screen.getByRole('button', { name: 'Save Mix like this as a station' }));
	await waitFor(() => expect(created).toHaveLength(1));
	expect((created[0]!.spec as { steps: unknown[] }).steps).toEqual([['menu', 0, 500, 'direct:1']]);
});
