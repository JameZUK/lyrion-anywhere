// Removing downloads (review M19): every way of removing asks first (saying what goes and how
// many tracks), runs once however often it is tapped, and a failure is shown, not thrown.
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import * as offline from '#lib/offline/index.svelte';
import type { DownloadManager } from '#lib/offline/manager.svelte';
import type { OfflineStore } from '#lib/offline/store';
import type { RowAction } from '#lib/components/RowMenu.svelte';
import DownloadBar from '#lib/components/DownloadBar.svelte';
import ArtistDownload from '#lib/components/ArtistDownload.svelte';
import DownloadsView from '#lib/components/DownloadsView.svelte';
import { key, stored } from './helpers/fake-downloads';

vi.mock('#lib/offline/index.svelte', async () => {
	const { DownloadManager } = await import('#lib/offline/manager.svelte');
	const { OfflinePlayer } = await import('#lib/offline/player.svelte');
	const { MemoryBackend, OfflineStore } = await import('#lib/offline/store');
	const { PlaySync } = await import('#lib/offline/plays.svelte');
	const offlineStore = new OfflineStore(new MemoryBackend());
	const hang = ((_u: unknown, init?: RequestInit) =>
		new Promise((_r, reject) => init?.signal?.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError'))))) as typeof fetch;
	const downloads = new DownloadManager({ store: offlineStore, authed: (fn) => fn('t'), fetch: hang, sleep: async () => {}, waitForWifi: () => false, capBytes: () => 0 });
	const offlinePlayer = new OfflinePlayer({ store: offlineStore, mediaSession: null });
	const plays = new PlaySync({ store: offlineStore, authed: (fn) => fn('t'), fetch: hang });
	return { offlineStore, downloads, offlinePlayer, plays };
});
vi.mock('#lib/state/app.svelte', () => ({
	api: { rpc: async () => ({}), art: () => Promise.reject(new Error('offline')) },
	app: { queueUp: vi.fn(), reach: 'online', canQueue: true },
	session: { current: null, subscribe: () => () => {} },
}));

const downloads = offline.downloads as DownloadManager;
const store = offline.offlineStore as OfflineStore;

const settle = async () => {
	for (let i = 0; i < 6; i++) await tick();
};

/** Unhandled rejections while a test runs fail it. */
const unhandled: unknown[] = [];
const onUnhandled = (e: unknown) => unhandled.push(e);
beforeAll(() => {
	HTMLDialogElement.prototype.showModal ??= function () {};
	process.on('unhandledRejection', onUnhandled);
});
afterAll(() => process.off('unhandledRejection', onUnhandled));

beforeEach(async () => {
	vi.restoreAllMocks();
	unhandled.length = 0;
	await downloads.setUser(null);
	await store.deleteUser('alice');
	localStorage.clear();
});
afterEach(async () => {
	await settle();
	expect(unhandled).toEqual([]);
	document.querySelectorAll('dialog').forEach((d) => d.remove());
});

async function save(list: Partial<ReturnType<typeof stored>>[]) {
	for (const t of list) {
		const s = stored({ user: 'alice', ...t });
		await store.putTrack(s);
		await store.putChunk('alice', s.key, 0, new Blob(['x']));
	}
	await downloads.setUser('alice', true);
}

function deferred<T>() {
	let resolve!: (v: T) => void;
	let reject!: (e: unknown) => void;
	const promise = new Promise<T>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

const openDialog = () => document.querySelector<HTMLDialogElement>('dialog[data-confirm-dialog]');
/** The confirm, once it is there. */
const dialogShown = () =>
	waitFor(() => {
		const d = openDialog();
		if (!d) throw new Error('no confirm dialog');
		return d;
	});
async function answer(yes: boolean) {
	const d = await dialogShown();
	await fireEvent.click(d.querySelector(yes ? '[data-confirm-ok]' : '[data-confirm-cancel]')!);
	await settle();
}
const removeAction = (c: { actions: () => RowAction[] }) => c.actions().find((a) => a.id === 'remove-download');

describe('album or playlist page (DownloadBar)', () => {
	const TRACKS = [
		{ id: 11, albumId: 7 },
		{ id: 12, albumId: 7 },
	];
	async function bar() {
		await save([
			{ key: key(11), id: 11, albumId: 7 },
			{ key: key(12), id: 12, albumId: 7 },
		]);
		const r = render(DownloadBar, { tracks: TRACKS, ref: 'album:7', name: 'An <b>Album</b>' });
		await settle();
		return r;
	}

	test('Remove download asks first, naming the album and the tracks; cancel removes nothing', async () => {
		const spy = vi.spyOn(downloads, 'removeAlbum');
		const r = await bar();
		removeAction(r.component)!.run();
		await settle();
		const d = openDialog()!;
		expect(d).not.toBeNull();
		expect(d.textContent).toContain('Remove the download of “An <b>Album</b>”?');
		expect(d.textContent).toContain('2 downloaded tracks');
		expect(d.querySelector('b')).toBeNull();
		await answer(false);
		expect(spy).not.toHaveBeenCalled();
		expect(await store.listTracks('alice')).toHaveLength(2);
		removeAction(r.component)!.run();
		await answer(true);
		await waitFor(async () => expect(await store.listTracks('alice')).toEqual([]));
		expect(spy).toHaveBeenCalledTimes(1);
	});

	test('a second activation while removing runs nothing more', async () => {
		const gate = deferred<{ removed: string[]; kept: number }>();
		const spy = vi.spyOn(downloads, 'removeAlbum').mockReturnValue(gate.promise);
		const r = await bar();
		removeAction(r.component)!.run();
		removeAction(r.component)!.run();
		await settle();
		expect(document.querySelectorAll('dialog[data-confirm-dialog]')).toHaveLength(1);
		await answer(true);
		// While it runs, the action is off and does nothing.
		const a = removeAction(r.component)!;
		expect(a.disabled).toBe(true);
		a.run();
		await settle();
		expect(openDialog()).toBeNull();
		gate.resolve({ removed: [], kept: 0 });
		await settle();
		expect(spy).toHaveBeenCalledTimes(1);
	});

	test('a failed removal says so instead of throwing', async () => {
		vi.spyOn(downloads, 'removeAlbum').mockRejectedValue(new DOMException('quota', 'UnknownError'));
		const r = await bar();
		removeAction(r.component)!.run();
		await answer(true);
		expect((await screen.findByRole('alert')).textContent).toBe('Couldn’t remove the download. Try again.');
		expect(removeAction(r.component)!.disabled).toBeFalsy();
	});
});

describe('artist page (ArtistDownload)', () => {
	const ALBUMS = [
		{ id: 7, album: 'One' },
		{ id: 8, album: 'Two' },
	];
	async function artist() {
		await save([
			{ key: key(1), id: 1, albumId: 7, refs: ['album:7'] },
			{ key: key(2), id: 2, albumId: 7, refs: ['album:7'] },
			{ key: key(3), id: 3, albumId: 8, refs: ['album:8'] },
		]);
		const r = render(ArtistDownload, { albums: ALBUMS, name: 'Band' });
		await settle();
		return r;
	}

	test('Remove downloads asks first, with the albums and tracks; cancel removes nothing', async () => {
		const spy = vi.spyOn(downloads, 'removeAlbum');
		const r = await artist();
		removeAction(r.component)!.run();
		await settle();
		const d = openDialog()!;
		expect(d.textContent).toContain('Remove the downloads of 2 albums by Band?');
		expect(d.textContent).toContain('3 downloaded tracks');
		await answer(false);
		expect(spy).not.toHaveBeenCalled();
		removeAction(r.component)!.run();
		await answer(true);
		await waitFor(async () => expect(await store.listTracks('alice')).toEqual([]));
		expect(spy).toHaveBeenCalledTimes(2);
	});

	test('a second activation while the loop runs starts no second loop', async () => {
		const gate = deferred<{ removed: string[]; kept: number }>();
		const spy = vi.spyOn(downloads, 'removeAlbum').mockReturnValue(gate.promise);
		const r = await artist();
		removeAction(r.component)!.run();
		await answer(true);
		const a = removeAction(r.component)!;
		expect(a.disabled).toBe(true);
		a.run();
		await settle();
		expect(openDialog()).toBeNull();
		gate.resolve({ removed: [], kept: 0 });
		await settle();
		expect(spy).toHaveBeenCalledTimes(2); // once per album, one loop
	});

	test('a failed removal says so instead of throwing', async () => {
		vi.spyOn(downloads, 'removeAlbum').mockRejectedValue(new Error('IndexedDB broke'));
		const r = await artist();
		removeAction(r.component)!.run();
		await answer(true);
		expect((await screen.findByRole('alert')).textContent).toBe('Couldn’t remove the downloads. Try again.');
	});
});

describe('Downloads page', () => {
	async function view() {
		await store.putGroup({ user: 'alice', id: 'playlist:3', name: 'Mix', ids: [3], savedAt: 1 });
		await save([
			{ key: key(1), id: 1, title: 'First', album: 'Album A', albumId: 7, refs: ['album:7'] },
			{ key: key(2), id: 2, title: 'Second', album: 'Album A', albumId: 7, refs: ['album:7'] },
			{ key: key(3), id: 3, title: 'Mixed', album: 'Album B', albumId: 8, refs: ['playlist:3'] },
		]);
		const r = render(DownloadsView);
		await settle();
		return r;
	}

	test('removing an album asks first; cancel removes nothing', async () => {
		const spy = vi.spyOn(downloads, 'removeAlbum');
		await view();
		await fireEvent.click(screen.getByRole('button', { name: 'Remove Album A' }));
		const d = await dialogShown();
		expect(d.textContent).toContain('Remove the download of “Album A”?');
		expect(d.textContent).toContain('2 downloaded tracks');
		await answer(false);
		expect(spy).not.toHaveBeenCalled();
		await fireEvent.click(screen.getByRole('button', { name: 'Remove Album A' }));
		await answer(true);
		await waitFor(() => expect(spy).toHaveBeenCalledWith(7));
	});

	test('removing a playlist or a single track asks first too', async () => {
		const pl = vi.spyOn(downloads, 'removePlaylist');
		const tr = vi.spyOn(downloads, 'removeTracks');
		await view();
		await fireEvent.click(screen.getByRole('button', { name: 'Remove Mix' }));
		expect((await dialogShown()).textContent).toContain('Remove the download of “Mix”?');
		await answer(false);
		expect(pl).not.toHaveBeenCalled();
		await fireEvent.click(screen.getByRole('button', { name: 'Show tracks of Album A' }));
		await fireEvent.click(screen.getByRole('button', { name: 'Remove First' }));
		expect((await dialogShown()).textContent).toContain('Remove the download of “First”?');
		await answer(false);
		expect(tr).not.toHaveBeenCalled();
		await fireEvent.click(screen.getByRole('button', { name: 'Remove First' }));
		await answer(true);
		await waitFor(() => expect(tr).toHaveBeenCalledWith([key(1)]));
	});

	test('while one removal runs, the remove buttons are off and a second does nothing', async () => {
		const gate = deferred<{ removed: string[]; kept: number }>();
		const spy = vi.spyOn(downloads, 'removeAlbum').mockReturnValue(gate.promise);
		await view();
		await fireEvent.click(screen.getByRole('button', { name: 'Remove Album A' }));
		await answer(true);
		const btn = screen.getByRole('button', { name: 'Remove Album A' }) as HTMLButtonElement;
		expect(btn.disabled).toBe(true);
		expect((screen.getByRole('button', { name: 'Remove Mix' }) as HTMLButtonElement).disabled).toBe(true);
		gate.resolve({ removed: [], kept: 0 });
		await settle();
		expect(spy).toHaveBeenCalledTimes(1);
	});

	test('a failed removal says so instead of throwing', async () => {
		vi.spyOn(downloads, 'removePlaylist').mockRejectedValue(new Error('IndexedDB broke'));
		vi.spyOn(downloads, 'removeAll').mockRejectedValue(new Error('IndexedDB broke'));
		await view();
		await fireEvent.click(screen.getByRole('button', { name: 'Remove Mix' }));
		await answer(true);
		expect((await screen.findByRole('alert')).textContent).toBe('Couldn’t remove the download. Try again.');
		await fireEvent.click(screen.getByRole('button', { name: 'Remove all…' }));
		await fireEvent.click(screen.getByRole('button', { name: 'Remove all' }));
		await waitFor(() => expect(screen.getByRole('alert').textContent).toBe('Couldn’t remove your downloads. Try again.'));
	});
});
