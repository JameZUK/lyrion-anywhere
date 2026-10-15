// Download UI: album page button states, track marks, the Downloads page (hostile metadata
// shown as text), offline mode from the "can't reach the server" screen, and the sign-out
// prompt about downloads.
import { fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import * as offline from '$lib/offline/index.svelte';
import type { DownloadManager } from '$lib/offline/manager.svelte';
import type { PlaySync } from '$lib/offline/plays.svelte';
import type { OfflineStore } from '$lib/offline/store';
import { key, stored } from './helpers/fake-downloads';
import { choose } from './helpers/menu';

vi.mock('$lib/offline/index.svelte', async () => {
	const { DownloadManager } = await import('$lib/offline/manager.svelte');
	const { OfflinePlayer } = await import('$lib/offline/player.svelte');
	const { MemoryBackend, OfflineStore } = await import('$lib/offline/store');
	const offlineStore = new OfflineStore(new MemoryBackend());
	// The server never answers (until aborted), so downloads stay in progress.
	const hang = ((_u: unknown, init?: RequestInit) =>
		new Promise((_r, reject) => init?.signal?.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError'))))) as typeof fetch;
	const control = { fetch: hang, hang, cellular: false, cap: 0 };
	const downloads = new DownloadManager({
		store: offlineStore,
		authed: (fn) => fn('t'),
		fetch: (...a) => control.fetch(...a),
		sleep: async () => {},
		waitForWifi: () => control.cellular,
		capBytes: () => control.cap,
	});
	const offlinePlayer = new OfflinePlayer({ store: offlineStore, mediaSession: null });
	const { PlaySync } = await import('$lib/offline/plays.svelte');
	const plays = new PlaySync({ store: offlineStore, authed: (fn) => fn('t'), fetch: (...a) => control.fetch(...a) });
	return { offlineStore, downloads, offlinePlayer, control, plays };
});

const ALBUM = { id: 7, album: 'An <b>Album</b>', artist: 'Band', artwork_track_id: '11' };
const TRACKS = [
	{ id: 11, title: 'One', tracknum: 1, duration: 100, artist: 'Band' },
	{ id: 12, title: 'Two', tracknum: 2, duration: 100, artist: 'Band' },
	{ id: 99, title: 'Radio', remote: 1 },
];
vi.mock('$lib/state/app.svelte', () => ({
	api: {
		rpc: async (_p: string, c: unknown[]) => (c[0] === 'albums' ? { albums_loop: [ALBUM] } : { titles_loop: TRACKS }),
		art: () => Promise.reject(new Error('offline')),
	},
	app: { queueUp: vi.fn(), reach: 'online', canQueue: true },
	session: { current: null, subscribe: () => () => {} },
}));
vi.mock('$app/state', () => ({ page: { params: { id: '7' }, url: new URL('http://app.example/#/album/7') } }));

const downloads = offline.downloads as DownloadManager;
const plays = offline.plays as PlaySync;
const store = offline.offlineStore as OfflineStore;
const control = (offline as unknown as { control: { fetch: typeof fetch; hang: typeof fetch; cellular: boolean; cap: number } }).control;

const HOSTILE = ['<img src=x onerror=alert(1)>', '<script>alert(1)</script>', '"><svg onload=alert(1)>', "javascript:alert('x')"];

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
	HTMLDialogElement.prototype.showModal ??= function () {};
});

beforeEach(async () => {
	await downloads.setUser(null);
	await plays.setUser(null);
	for (const u of ['alice', 'bob']) await store.deleteUser(u);
	localStorage.clear();
	control.fetch = control.hang;
	control.cellular = false;
	control.cap = 0;
});

async function saveTracks(user: string, list: Partial<ReturnType<typeof stored>>[]) {
	for (const t of list) {
		const s = stored({ user, ...t });
		await store.putTrack(s);
		await store.putChunk(user, s.key, 0, new Blob(['x']));
	}
}

async function albumPage() {
	const { default: Page } = await import('../src/routes/album/[id]/+page.svelte');
	const r = render(Page);
	await screen.findByRole('heading', { name: ALBUM.album });
	return r;
}

/** ⋯ › Download as… › a format (Wave 3.5: the format is no longer a select in the header). */
async function downloadAs(format: string) {
	await choose('More options for An <b>Album</b>', 'Download as…');
	await fireEvent.click(document.querySelector(`[data-format-picker] [data-format="${format}"]`)!);
}

test('album page: Download in the usual format when nothing is downloaded; Download as… offers each format', async () => {
	await downloads.setUser('alice', true);
	await albumPage();
	const btn = screen.getByRole('button', { name: 'Download' });
	expect(btn).not.toHaveProperty('disabled', true);
	expect(screen.queryByLabelText('Download format')).toBeNull();
	expect(screen.queryByText('Downloaded')).toBeNull();
	await choose('More options for An <b>Album</b>', 'Download as…');
	const picker = document.querySelector('[data-format-picker]')!;
	expect([...picker.querySelectorAll<HTMLElement>('[data-format]')].map((b) => b.dataset.format)).toEqual(['auto', 'original', 'mp3', 'opus']);
});

test('album page: progress while downloading, then pause', async () => {
	await downloads.setUser('alice', true);
	await albumPage();
	await downloadAs('mp3');
	await vi.waitFor(() => expect(downloads.items).toHaveLength(2));
	await tick();
	// Library tracks only: the radio track is never queued.
	expect(downloads.items.map((i) => [i.id, i.choice, i.albumId])).toEqual([
		[11, 'mp3', 7],
		[12, 'mp3', 7],
	]);
	expect(screen.getByRole('status').textContent).toContain('Downloading 0 of 2');
	expect(screen.getByRole('progressbar', { name: 'Download progress' })).toBeTruthy();
	// One mark per meaning: tracks waiting their turn have none (the line above says it).
	expect(screen.queryAllByLabelText('Waiting to download')).toHaveLength(0);
	await fireEvent.click(screen.getByRole('button', { name: 'Pause' }));
	expect(screen.getByRole('status').textContent).toContain('Download paused');
	expect(screen.getByRole('button', { name: 'Resume' })).toBeTruthy();
	await downloads.cancelAll(7);
});

test('album page: Downloaded ✓ and Remove download once every track is here', async () => {
	await saveTracks('alice', [
		{ key: key(11), id: 11, albumId: 7 },
		{ key: key(12), id: 12, albumId: 7 },
	]);
	await downloads.setUser('alice', true);
	await albumPage();
	expect(screen.getByRole('status').textContent).toContain('Downloaded');
	expect(screen.getAllByLabelText('Downloaded').length).toBe(2); // the track marks
	expect(screen.queryByRole('button', { name: 'Download' })).toBeNull();
	// Destructive: only in ⋯.
	expect(screen.queryByRole('button', { name: 'Remove download' })).toBeNull();
	await choose('More options for An <b>Album</b>', 'Remove download');
	await fireEvent.click(await vi.waitFor(() => { const b = document.querySelector('dialog[data-confirm-dialog] [data-confirm-ok]'); if (!b) throw new Error('no confirm'); return b; })); // it asks first
	await vi.waitFor(() => expect(screen.getByRole('button', { name: 'Download' })).toBeTruthy());
	expect(await store.listTracks('alice')).toEqual([]);
});

test('album page: some tracks here offers the rest; offline the button is off', async () => {
	await saveTracks('alice', [{ key: key(11), id: 11, albumId: 7 }]);
	await downloads.setUser('alice', false);
	await albumPage();
	expect(screen.getByText('1 of 2 downloaded')).toBeTruthy();
	expect((screen.getByRole('button', { name: 'Download the rest' }) as HTMLButtonElement).disabled).toBe(true);
});

test('album page: on mobile data with Wi-Fi only, the download waits; Download now starts it', async () => {
	control.cellular = true;
	await downloads.setUser('alice', true);
	await albumPage();
	await fireEvent.click(screen.getByRole('button', { name: 'Download' }));
	await vi.waitFor(() => expect(screen.getByRole('status').textContent).toContain('Waiting for Wi-Fi (2 tracks)'));
	expect(screen.queryAllByLabelText('Waiting for Wi-Fi')).toHaveLength(0);
	expect(downloads.items.every((i) => i.status === 'queued')).toBe(true);
	await fireEvent.click(screen.getByRole('button', { name: 'Download now' }));
	await vi.waitFor(() => expect(screen.getByRole('status').textContent).toContain('Downloading 0 of 2'));
	expect(downloads.items.every((i) => i.cellular && i.status === 'active')).toBe(true);
	await downloads.cancelAll();
});

test('album page: a download past the storage limit is refused with what it needs and what is free', async () => {
	control.cap = 1024 * 1024;
	await downloads.setUser('alice', true);
	await albumPage();
	await downloadAs('mp3');
	// Two 100 s tracks at 320 kb/s: about 7.6 MB, with 1 MB allowed.
	const alert = await screen.findByRole('alert');
	expect(alert.textContent).toBe(
		'Not enough room under your storage limit: this needs about 7.6 MB, and 1.0 MB of 1.0 MB is free. Remove some downloads, or raise the limit in Settings.',
	);
	expect(downloads.items).toEqual([]);
});

test('album page: a single track downloads on its own from its row menu', async () => {
	await downloads.setUser('alice', true);
	await albumPage();
	await fireEvent.click(screen.getByRole('button', { name: 'More options for Two' }));
	await fireEvent.click(screen.getByRole('menuitem', { name: 'Download' }));
	await vi.waitFor(() => expect(downloads.items.map((i) => [i.id, i.refs])).toEqual([[12, ['track']]]));
	// The radio track has no download entry.
	await fireEvent.click(screen.getByRole('button', { name: 'More options for Radio' }));
	expect(screen.queryByRole('menuitem', { name: 'Download' })).toBeNull();
	await downloads.cancelAll();
});

test('album page: removing a track a playlist needs keeps it and says so', async () => {
	await saveTracks('alice', [{ key: key(12), id: 12, albumId: 7, refs: ['album:7', 'playlist:3'] }]);
	await downloads.setUser('alice', true);
	await albumPage();
	await fireEvent.click(screen.getByRole('button', { name: 'More options for Two' }));
	await fireEvent.click(screen.getByRole('menuitem', { name: 'Remove download' }));
	expect(await screen.findByText('This track stays: a downloaded playlist still needs it.')).toBeTruthy();
	expect((await store.getTrack('alice', key(12)))?.refs).toEqual(['playlist:3']);
});

test('Downloads page: playlists are their own group; removing one keeps tracks an album needs', async () => {
	await saveTracks('alice', [
		{ key: key(1), id: 1, title: 'Shared', album: 'Album A', albumId: 7, refs: ['album:7', 'playlist:3'] },
		{ key: key(2), id: 2, title: 'Only in mix', album: 'Album B', albumId: 8, refs: ['playlist:3'] },
	]);
	await store.putGroup({ user: 'alice', id: 'playlist:3', name: '<b>Mix</b>', ids: [2, 1], savedAt: 1 });
	await downloads.setUser('alice', true);
	const { default: View } = await import('$lib/components/DownloadsView.svelte');
	const { container } = render(View);
	await tick();
	expect(screen.getByRole('heading', { name: 'Playlists' })).toBeTruthy();
	expect(container.textContent).toContain('<b>Mix</b>');
	expect(container.querySelector('b')).toBeNull();
	// The album list shows only what was downloaded with its album.
	expect(container.textContent).toContain('Album A');
	expect(container.textContent).not.toContain('Album B');
	await fireEvent.click(screen.getByRole('button', { name: 'Show tracks of <b>Mix</b>' }));
	const order = screen.getAllByRole('button', { name: /^Play (Shared|Only in mix)$/ }).map((b) => b.getAttribute('aria-label'));
	expect(order.slice(0, 2)).toEqual(['Play Only in mix', 'Play Shared']);
	await fireEvent.click(screen.getByRole('button', { name: 'Remove <b>Mix</b>' }));
	await fireEvent.click(await vi.waitFor(() => { const b = document.querySelector('dialog[data-confirm-dialog] [data-confirm-ok]'); if (!b) throw new Error('no confirm'); return b; })); // it asks first
	await vi.waitFor(() => expect(screen.queryByRole('heading', { name: 'Playlists' })).toBeNull());
	expect(screen.getByText(/1 track stays on this device/)).toBeTruthy();
	expect((await store.listTracks('alice')).map((t) => [t.id, t.refs])).toEqual([[1, ['album:7']]]);
	expect(await store.listGroups('alice')).toEqual([]);
});

test('Downloads page: waiting for Wi-Fi, with Download now; the storage limit is shown', async () => {
	control.cellular = true;
	localStorage.setItem('anywhere.downloads', JSON.stringify({ capBytes: 2 * 1024 ** 3 }));
	await downloads.setUser('alice', true);
	await downloads.request([{ id: 5 }, { id: 6 }]);
	const { default: View } = await import('$lib/components/DownloadsView.svelte');
	render(View);
	await tick();
	expect(screen.getByTestId('wifi-wait').textContent).toContain('Waiting for Wi-Fi: 2 downloads will start on Wi-Fi.');
	expect(screen.getAllByText('Waiting for Wi-Fi').length).toBe(2);
	expect(screen.getByTestId('storage-limit').textContent).toContain('2.00 GB limit');
	await fireEvent.click(screen.getByRole('button', { name: 'Download now' }));
	await vi.waitFor(() => expect(screen.queryByTestId('wifi-wait')).toBeNull());
	await downloads.cancelAll();
});

test('Downloads page shows hostile titles as text, with sizes, and removes', async () => {
	await saveTracks(
		'alice',
		HOSTILE.map((h, i) => ({ key: key(i + 1), id: i + 1, title: h, artist: h, album: h, albumId: 7, size: 1024 * 1024 })),
	);
	await downloads.setUser('alice', true);
	const { default: View } = await import('$lib/components/DownloadsView.svelte');
	const { container } = render(View);
	await tick();
	expect(container.querySelectorAll('script, img, iframe, object, embed').length).toBe(0);
	for (const el of container.querySelectorAll('*')) for (const a of el.attributes) expect(a.name.startsWith('on'), a.name).toBe(false);
	expect(container.textContent).toContain('4 tracks, 4.0 MB');
	expect(container.textContent).toContain(HOSTILE[0]);
	await fireEvent.click(screen.getByRole('button', { name: `Show tracks of ${HOSTILE[0]}` }));
	for (const h of HOSTILE) expect(screen.getAllByRole('button', { name: `Play ${h}` }).length).toBeGreaterThan(0);
	await fireEvent.click(screen.getByRole('button', { name: 'Remove all…' }));
	await fireEvent.click(screen.getByRole('button', { name: 'Remove all' }));
	await vi.waitFor(() => expect(container.textContent).toContain('Nothing downloaded yet'));
	expect(await store.listTracks('alice')).toEqual([]);
});

test('Downloads page only lists the signed-in user’s downloads', async () => {
	await saveTracks('alice', [{ key: key(1), title: 'Alice song', album: 'Alice album' }]);
	await saveTracks('bob', [{ key: key(2), title: 'Bob song', album: 'Bob album' }]);
	await downloads.setUser('bob', true);
	const { default: View } = await import('$lib/components/DownloadsView.svelte');
	const { container } = render(View);
	await tick();
	expect(container.textContent).toContain('Bob album');
	expect(container.textContent).not.toContain('Alice album');
});

test('Downloads page says how many plays are waiting to sync, for this user only', async () => {
	await saveTracks('alice', [{ key: key(1), id: 1, title: 'Song', album: 'Album' }]);
	for (const p of await store.listPlays('alice')) await store.deletePlay('alice', p.id);
	await plays.record('alice', { id: 1, key: key(1) }, Date.now() - 600_000, 250);
	await plays.record('alice', { id: 1, key: key(1) }, Date.now() - 300_000, 250);
	await plays.record('bob', { id: 1, key: key(1) }, Date.now() - 300_000, 250);
	await downloads.setUser('alice', false);
	await plays.setUser('alice', false);
	const { default: View } = await import('$lib/components/DownloadsView.svelte');
	const { container } = render(View);
	await tick();
	expect(container.textContent).toContain('2 plays waiting to sync: they’ll be sent when you’re back online.');
	// Removing every download keeps the plays: they are listening history, not downloads.
	await fireEvent.click(screen.getByRole('button', { name: 'Remove all…' }));
	await fireEvent.click(screen.getByRole('button', { name: 'Remove all' }));
	await vi.waitFor(() => expect(container.textContent).toContain('Nothing downloaded yet'));
	expect((await store.listPlays('alice')).length).toBe(2);
	for (const u of ['alice', 'bob']) for (const p of await store.listPlays(u)) await store.deletePlay(u, p.id);
});

test('offline splash offers “Play your downloads” for the account last signed in here', async () => {
	await saveTracks('alice', [{ key: key(1), title: 'Offline song', album: 'Offline album' }]);
	localStorage.setItem('anywhere.offlineUser', 'alice');
	const { default: OfflineMode } = await import('$lib/components/OfflineMode.svelte');
	render(OfflineMode);
	expect(screen.getByText('Can’t reach your music server. Trying again…')).toBeTruthy();
	const play = await screen.findByRole('button', { name: 'Play your downloads' });
	await fireEvent.click(play);
	expect(await screen.findByText('Offline album')).toBeTruthy();
	expect(screen.getByText(/You’re offline/)).toBeTruthy();
	expect(downloads.online).toBe(false);
});

test('offline splash offers nothing without a remembered account (e.g. after signing out)', async () => {
	await saveTracks('alice', [{ key: key(1) }]);
	const { default: OfflineMode } = await import('$lib/components/OfflineMode.svelte');
	render(OfflineMode);
	await tick();
	await new Promise((r) => setTimeout(r, 20));
	expect(screen.queryByRole('button', { name: 'Play your downloads' })).toBeNull();
});

test('sign-out asks about downloads; keeping is the default and removal one tap', async () => {
	const { default: SignOut } = await import('$lib/components/SignOut.svelte');
	const onsignout = vi.fn();
	const r = render(SignOut, { count: 3, bytes: 3 * 1024 * 1024, onsignout });
	await fireEvent.click(screen.getByRole('button', { name: 'Sign out' }));
	expect(onsignout).not.toHaveBeenCalled();
	expect(screen.getByText(/Downloads stay on this device until/)).toBeTruthy();
	const keep = screen.getByRole('button', { name: 'Keep downloads and sign out' });
	expect(keep.className).toContain('primary');
	await fireEvent.click(screen.getByRole('button', { name: 'Remove downloads and sign out' }));
	expect(onsignout).toHaveBeenCalledWith(true);
	r.unmount();

	const again = vi.fn();
	render(SignOut, { count: 3, onsignout: again });
	await fireEvent.click(screen.getByRole('button', { name: 'Sign out' }));
	await fireEvent.click(screen.getByRole('button', { name: 'Keep downloads and sign out' }));
	expect(again).toHaveBeenCalledWith(false);
});

test('sign-out with nothing downloaded does not ask', async () => {
	const { default: SignOut } = await import('$lib/components/SignOut.svelte');
	const onsignout = vi.fn();
	render(SignOut, { count: 0, onsignout });
	await fireEvent.click(screen.getByRole('button', { name: 'Sign out' }));
	expect(onsignout).toHaveBeenCalledWith(false);
});
