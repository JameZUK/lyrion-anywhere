// "Recently played" in Library is the user's own (their history on the gateway), never LMS's
// server-wide `recentlyplayed`, which would show what everyone else played.
import { render, waitFor } from '@testing-library/svelte';

vi.mock('#lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const shelfCalls: [string, number][] = [];
vi.mock('#lib/state/me', () => ({
	me: {
		shelf: async (name: string, limit: number) => {
			shelfCalls.push([name, limit]);
			return [30, 10, 20];
		},
	},
}));
const fake = await import('./helpers/fake-app.svelte');
const AlbumBrowser = (await import('#lib/components/AlbumBrowser.svelte')).default;

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

test("recently played lists the user's own albums, newest first", async () => {
	fake.calls.length = 0;
	fake.handler.rpc = async (_p, c) =>
		c[0] === 'albums'
			? { albums_loop: [10, 20, 30].map((id) => ({ id, album: `Album ${id}`, artist: 'Some Artist' })), count: 3 }
			: {};
	const { findAllByText } = render(AlbumBrowser, { props: { sort: 'played' } });
	await waitFor(async () => expect((await findAllByText(/^Album \d+$/)).map((e) => e.textContent)).toEqual(['Album 30', 'Album 10', 'Album 20']));
	expect(shelfCalls[0]).toEqual(['recent', 50]);
	const albums = fake.calls.filter((c) => c.cmd[0] === 'albums').map((c) => c.cmd.join(' '));
	expect(albums.length).toBeGreaterThan(0);
	for (const a of albums) {
		expect(a).toContain('album_id:30,10,20');
		expect(a).not.toContain('recentlyplayed');
	}
});
