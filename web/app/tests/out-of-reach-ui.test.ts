// Controls that need the server, while it is out of reach or busy (2026-10-14): disabled
// consistently (Play and Download, LMS's More menus, Bring here and Back to this device), each
// pointing at the connection message to say why; what works without the server (removing a
// download) stays available.
import { fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import type { DownloadManager } from '$lib/offline/manager.svelte';
import { CONNECTION_ID, UNREACHABLE_TEXT } from '$lib/state/reachability';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
vi.mock('$lib/offline/index.svelte', async () => {
	const { DownloadManager } = await import('$lib/offline/manager.svelte');
	const { OfflinePlayer } = await import('$lib/offline/player.svelte');
	const { MemoryBackend, OfflineStore } = await import('$lib/offline/store');
	const { PlaySync } = await import('$lib/offline/plays.svelte');
	const offlineStore = new OfflineStore(new MemoryBackend());
	const hang = (() => new Promise(() => {})) as typeof fetch;
	const downloads = new DownloadManager({ store: offlineStore, authed: (fn) => fn('t'), fetch: hang, sleep: async () => {}, waitForWifi: () => false, capBytes: () => 0 });
	const offlinePlayer = new OfflinePlayer({ store: offlineStore, mediaSession: null });
	const plays = new PlaySync({ store: offlineStore, authed: (fn) => fn('t'), fetch: hang });
	return { offlineStore, downloads, offlinePlayer, plays };
});
vi.mock('$app/state', () => ({ page: { params: { id: '7' }, url: new URL('http://app.example/#/album/7') } }));

const fake = await import('./helpers/fake-app.svelte');
const offline = await import('$lib/offline/index.svelte');
const downloads = offline.downloads as DownloadManager;
const { device } = await import('$lib/player/device.svelte');
const { default: TrackList } = await import('$lib/components/TrackList.svelte');
const { default: PlayingElsewhere } = await import('$lib/components/PlayingElsewhere.svelte');
const { default: AlbumPage } = await import('../src/routes/album/[id]/+page.svelte');

const ME = '02:4c:41:00:00:01';
const LOUNGE = 'aa:bb:cc:dd:ee:01';
const TRACKS = [
	{ id: 11, title: 'One', tracknum: 1, duration: 100, artist: 'Band' },
	{ id: 12, title: 'Two', tracknum: 2, duration: 100, artist: 'Band' },
];

class NoObserver {
	observe() {}
	unobserve() {}
	disconnect() {}
	takeRecords() {
		return [];
	}
}
beforeAll(() => {
	vi.stubGlobal('IntersectionObserver', NoObserver);
	vi.stubGlobal('ResizeObserver', NoObserver);
	vi.stubGlobal('matchMedia', () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
});
afterAll(() => vi.unstubAllGlobals());

beforeEach(async () => {
	vi.stubGlobal('AudioWorkletNode', class {});
	vi.stubGlobal('Worker', class {});
	fake.calls.length = 0;
	fake.moves.length = 0;
	fake.app.deviceMac = ME;
	fake.app.players = [
		{ playerid: ME, name: 'Firefox on Linux' },
		{ playerid: LOUNGE, name: 'Lounge', isplaying: 1 },
	];
	fake.app.playerId = LOUNGE;
	fake.app.now = { mode: 'play', playlist_tracks: 6 };
	fake.app.reach = 'online';
	fake.handler.rpc = async (_p, c) => (c[0] === 'albums' ? { albums_loop: [{ id: 7, album: 'Record', artist: 'Band', artwork_track_id: '11' }] } : c[0] === 'titles' ? { titles_loop: TRACKS } : {});
	device.state = 'connected';
	device.mac = ME;
	await downloads.setUser(null);
	await downloads.setUser('alice', true);
});

/** Disabled, and saying why: pointing at the connection message. */
function explained(el: HTMLElement) {
	expect(el.getAttribute('aria-describedby')).toBe(CONNECTION_ID);
	expect(el.getAttribute('title')).toBe(UNREACHABLE_TEXT);
}

describe('the album page', () => {
	async function open() {
		const r = render(AlbumPage);
		await screen.findByRole('heading', { name: 'Record' });
		return r;
	}

	test('in reach: Play, More and Download all work, and say nothing extra', async () => {
		await open();
		for (const name of ['Play', 'More', 'Download']) {
			const b = screen.getByRole('button', { name }) as HTMLButtonElement;
			expect(b.disabled, name).toBe(false);
			expect(b.getAttribute('aria-describedby'), name).toBeNull();
		}
	});

	test('out of reach: Play, Play next, Add, More, Download and the format are all disabled, and say why', async () => {
		fake.app.reach = 'unreachable';
		await open();
		for (const name of ['Play', 'Play next', 'Add to queue', 'More', 'Download']) {
			const b = screen.getByRole('button', { name }) as HTMLButtonElement;
			expect(b.disabled, name).toBe(true);
			explained(b);
		}
		expect((screen.getByLabelText('Download format') as HTMLSelectElement).disabled).toBe(true);
		(screen.getByRole('button', { name: 'Download' }) as HTMLButtonElement).click();
		await tick();
		expect(downloads.items).toEqual([]);
	});

	test("out of reach: the track menu's Download and More… are unavailable and say why; nothing happens", async () => {
		fake.app.reach = 'unreachable';
		await open();
		await fireEvent.click(screen.getByRole('button', { name: 'More options for Two' }));
		const items = screen.getAllByRole('menuitem');
		expect(items.map((i) => [i.textContent?.trim(), i.getAttribute('aria-disabled')])).toEqual([
			['Play next', 'true'],
			['Add to queue', 'true'],
			['Download', 'true'],
			['More…', 'true'],
		]);
		for (const i of items) explained(i);
		await fireEvent.click(screen.getByRole('menuitem', { name: 'Download' }));
		await fireEvent.click(screen.getByRole('menuitem', { name: 'More…' }));
		await tick();
		expect(downloads.items).toEqual([]);
		expect(document.querySelector('dialog')).toBeNull();
	});

	test('busy counts the same as out of reach', async () => {
		fake.app.reach = 'busy';
		await open();
		const more = screen.getByRole('button', { name: 'More' }) as HTMLButtonElement;
		expect(more.disabled).toBe(true);
		expect(more.getAttribute('aria-describedby')).toBe(CONNECTION_ID);
	});
});

describe('the track menu', () => {
	test('removing a download works without the server', async () => {
		const onremove = vi.fn();
		const r = render(TrackList, { tracks: TRACKS, onplay: () => {}, onnext: () => {}, download: () => 'Remove download', ondownload: onremove, disabled: true });
		await fireEvent.click(r.getByRole('button', { name: 'More options for One' }));
		const remove = r.getByRole('menuitem', { name: 'Remove download' });
		expect(remove.getAttribute('aria-disabled')).toBeNull();
		await fireEvent.click(remove);
		expect(onremove).toHaveBeenCalledTimes(1);
	});
});

describe('the "Playing on …" strip', () => {
	test('in reach: Bring here and Back to this device work', () => {
		render(PlayingElsewhere);
		for (const name of ['Bring playback to this device', 'Back to this device']) {
			expect((screen.getByRole('button', { name }) as HTMLButtonElement).disabled).toBe(false);
		}
	});

	test('out of reach: both are disabled (they need the server), say why, and do nothing', async () => {
		fake.app.reach = 'unreachable';
		render(PlayingElsewhere);
		for (const name of ['Bring playback to this device', 'Back to this device']) {
			const b = screen.getByRole('button', { name }) as HTMLButtonElement;
			expect(b.disabled).toBe(true);
			explained(b);
			b.click(); // as a browser does: nothing, for a disabled button
			await tick();
		}
		expect(fake.app.playerId).toBe(LOUNGE);
		expect(fake.moves).toEqual([]);
	});
});
