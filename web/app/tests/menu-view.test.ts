// Menus must not restart (flicker, lose their place) when the page around them re-renders.
import { render } from '@testing-library/svelte';
import { tick } from 'svelte';

const calls: unknown[][] = [];
vi.mock('$lib/state/app.svelte', () => ({
	app: { playerId: '02:4c:41:00:00:01', soon() {}, queueVersion: 0 },
	api: {
		async rpc(_player: string, cmd: unknown[]) {
			calls.push(cmd);
			return { count: 1, item_loop: [{ text: `Item for ${String(cmd[2])}`, actions: { go: { cmd: ['x', 'items'], params: { item_id: '1' } } } }] };
		},
		art: async () => 'blob:x',
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

const source = (id: number) => (from: number, n: number) => ['trackinfo', 'items', from, n, `track_id:${id}`, 'menu:1'];
const settle = async () => {
	for (let i = 0; i < 5; i++) {
		await tick();
		await new Promise((r) => setTimeout(r, 0));
	}
};

// Regression: the page re-renders every 2 s (status refresh) and hands the menu a new but
// identical source; the menu restarted each time, so open menus flickered.
test('an identical source on re-render keeps the menu as it is', async () => {
	calls.length = 0;
	const { rerender, container } = render(MenuView, { start: source(9), title: 'Song' });
	await settle();
	expect(calls).toHaveLength(1);
	for (let i = 0; i < 5; i++) {
		await rerender({ start: source(9), title: 'Song' });
		await settle();
	}
	expect(calls, 'no reloads').toHaveLength(1);
	expect(container.textContent).toContain('Item for 0');
	// A new title alone just renames it.
	await rerender({ start: source(9), title: 'Song (live)' });
	await settle();
	expect(calls).toHaveLength(1);
	// A different menu (another track) starts over.
	await rerender({ start: source(10), title: 'Other' });
	await settle();
	expect(calls).toHaveLength(2);
});
