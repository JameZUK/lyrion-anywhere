// Downloading a playlist (SPEC §8.1) from its page: library tracks only, kept as a group
// with the playlist's name and order, and removed without touching tracks an album needs.
import { fireEvent, render, screen } from '@testing-library/svelte';
import * as offline from '$lib/offline/index.svelte';
import type { DownloadManager } from '$lib/offline/manager.svelte';
import { choose } from './helpers/menu';
import type { OfflineStore } from '$lib/offline/store';
import { key, stored } from './helpers/fake-downloads';

vi.mock('$lib/offline/index.svelte', async () => {
	const { DownloadManager } = await import('$lib/offline/manager.svelte');
	const { OfflinePlayer } = await import('$lib/offline/player.svelte');
	const { MemoryBackend, OfflineStore } = await import('$lib/offline/store');
	const offlineStore = new OfflineStore(new MemoryBackend());
	// The server never answers (until aborted), so downloads stay in progress.
	const hang = ((_u: unknown, init?: RequestInit) =>
		new Promise((_r, reject) => init?.signal?.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError'))))) as typeof fetch;
	const downloads = new DownloadManager({ store: offlineStore, authed: (fn) => fn('t'), fetch: hang, sleep: async () => {} });
	const offlinePlayer = new OfflinePlayer({ store: offlineStore, mediaSession: null });
	return { offlineStore, downloads, offlinePlayer };
});

const LIST = { id: 5, playlist: 'Road <i>trip</i>' };
const TRACKS = [
	{ id: 21, title: 'First', duration: 100, album_id: 9 },
	{ id: 22, title: 'Second', duration: 100 },
	{ id: 23, title: 'A stream', remote: 1 },
	{ id: 21, title: 'First again', duration: 100 },
];
vi.mock('$lib/state/app.svelte', () => ({
	api: {
		rpc: async (_p: string, c: unknown[]) => (c[1] === 'tracks' ? { playlisttracks_loop: TRACKS } : { playlists_loop: [LIST] }),
		art: () => Promise.reject(new Error('offline')),
	},
	app: { queueUp: vi.fn(), isAdmin: false },
	session: { current: null, subscribe: () => () => {} },
}));
vi.mock('$app/state', () => ({ page: { params: { id: '5' }, url: new URL('http://app.example/#/playlist/5') } }));

const downloads = offline.downloads as DownloadManager;
const store = offline.offlineStore as OfflineStore;

beforeEach(async () => {
	await downloads.setUser(null);
	await store.deleteUser('alice');
	localStorage.clear();
});

async function playlistPage() {
	const { default: Page } = await import('../src/routes/playlist/[id]/+page.svelte');
	const r = render(Page);
	await screen.findByRole('heading', { name: LIST.playlist });
	return r;
}

test('Download queues the playlist’s library tracks as a group with its name and order', async () => {
	await downloads.setUser('alice', true);
	await playlistPage();
	await fireEvent.click(screen.getByRole('button', { name: 'Download' }));
	await vi.waitFor(() => expect(downloads.items.map((i) => [i.id, i.refs])).toEqual([
		[21, ['playlist:5']],
		[22, ['playlist:5']],
	]));
	expect(downloads.groups.map((g) => [g.id, g.name, g.ids])).toEqual([['playlist:5', LIST.playlist, [21, 22, 21]]]);
	expect(screen.getByRole('status').textContent).toContain('Downloading 0 of 2');
	await downloads.cancelAll();
});

test('tracks already downloaded with an album join the playlist without being fetched; removal keeps them', async () => {
	await store.putTrack(stored({ key: key(21), id: 21, albumId: 9 }));
	await store.putTrack(stored({ key: key(22), id: 22, albumId: 8, refs: ['playlist:5'] }));
	await store.putGroup({ user: 'alice', id: 'playlist:5', name: LIST.playlist, ids: [21, 22], savedAt: 1 });
	await downloads.setUser('alice', true);
	await playlistPage();
	// 21 is here only for its album: the playlist isn't fully downloaded yet.
	expect(screen.getByText('2 of 2 downloaded')).toBeTruthy();
	await fireEvent.click(screen.getByRole('button', { name: 'Download' }));
	await vi.waitFor(() => expect(screen.getByRole('status').textContent).toContain('Downloaded'));
	expect(downloads.items).toEqual([]);
	expect(screen.getAllByLabelText('Downloaded').length).toBe(3); // rows 21, 22 and 21 again
	// Destructive: only in the page's ⋯.
	await choose('More options for Road <i>trip</i>', 'Remove download');
	// 21 stays for its album, so the playlist offers the rest again.
	await vi.waitFor(() => expect(screen.getByRole('button', { name: 'Download the rest' })).toBeTruthy());
	expect(await screen.findByText('1 track stays: another download still needs it.')).toBeTruthy();
	expect((await store.listTracks('alice')).map((t) => [t.id, t.refs])).toEqual([[21, ['album:9']]]);
	expect(await store.listGroups('alice')).toEqual([]);
});

test('offline, the Download button is off', async () => {
	await downloads.setUser('alice', false);
	await playlistPage();
	expect((screen.getByRole('button', { name: 'Download' }) as HTMLButtonElement).disabled).toBe(true);
});
