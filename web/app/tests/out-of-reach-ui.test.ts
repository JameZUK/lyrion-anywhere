// Controls that need the server, while it is out of reach or busy (2026-10-14): disabled
// consistently (Play, Shuffle and Download, the header's ⋯ items, LMS's More menus, moving
// playback in Play on…), each pointing at the connection message to say why; what works
// without the server (removing a download) stays available.
import { fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { stubWebAudio } from './helpers/web-audio';
import type { DownloadManager } from '#lib/offline/manager.svelte';
import { CONNECTION_ID, UNREACHABLE_TEXT } from '#lib/state/reachability';

vi.mock('#lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
vi.mock('#lib/offline/index.svelte', async () => {
	const { DownloadManager } = await import('#lib/offline/manager.svelte');
	const { OfflinePlayer } = await import('#lib/offline/player.svelte');
	const { MemoryBackend, OfflineStore } = await import('#lib/offline/store');
	const { PlaySync } = await import('#lib/offline/plays.svelte');
	const offlineStore = new OfflineStore(new MemoryBackend());
	const hang = (() => new Promise(() => {})) as typeof fetch;
	const downloads = new DownloadManager({ store: offlineStore, authed: (fn) => fn('t'), fetch: hang, sleep: async () => {}, waitForWifi: () => false, capBytes: () => 0 });
	const offlinePlayer = new OfflinePlayer({ store: offlineStore, mediaSession: null });
	const plays = new PlaySync({ store: offlineStore, authed: (fn) => fn('t'), fetch: hang });
	return { offlineStore, downloads, offlinePlayer, plays };
});
vi.mock('$app/state', () => ({ page: { params: { id: '7' }, url: new URL('http://app.example/#/album/7') } }));

const fake = await import('./helpers/fake-app.svelte');
const offline = await import('#lib/offline/index.svelte');
const downloads = offline.downloads as DownloadManager;
const { device } = await import('#lib/player/device.svelte');
const { default: TrackList } = await import('#lib/components/TrackList.svelte');
const { default: PlayerSwitcher } = await import('#lib/components/PlayerSwitcher.svelte');
const { openMenu } = await import('./helpers/menu');
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
	stubWebAudio();
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

	const HEADER = 'More options for Record';

	test('in reach: Play, Shuffle, Download and the ⋯ items all work, and say nothing extra', async () => {
		await open();
		for (const name of ['Play', 'Shuffle', 'Download']) {
			const b = screen.getByRole('button', { name }) as HTMLButtonElement;
			expect(b.disabled, name).toBe(false);
			expect(b.getAttribute('aria-describedby'), name).toBeNull();
		}
		const menu = await openMenu(HEADER);
		for (const item of menu.querySelectorAll('[role=menuitem]')) {
			expect(item.getAttribute('aria-disabled'), item.textContent!).toBeNull();
			expect(item.getAttribute('aria-describedby'), item.textContent!).toBeNull();
		}
	});

	test('out of reach: Play, Shuffle, Download, and Play next, Add, Download as… and More… in ⋯, are all unavailable, and say why', async () => {
		fake.app.reach = 'unreachable';
		await open();
		for (const name of ['Play', 'Shuffle', 'Download']) {
			const b = screen.getByRole('button', { name }) as HTMLButtonElement;
			expect(b.disabled, name).toBe(true);
			explained(b);
		}
		// The ⋯ itself still opens: its items say what can't be done now.
		const menu = await openMenu(HEADER);
		const items = [...menu.querySelectorAll<HTMLElement>('[role=menuitem]')];
		expect(items.map((i) => i.textContent?.trim())).toEqual(['Play next', 'Add to queue', 'Download as…', 'More…']);
		for (const i of items) {
			expect(i.getAttribute('aria-disabled'), i.textContent!).toBe('true');
			explained(i);
		}
		(screen.getByRole('button', { name: 'Download' }) as HTMLButtonElement).click();
		for (const i of items) await fireEvent.click(i);
		await tick();
		expect(downloads.items).toEqual([]);
		expect(document.querySelector('dialog')).toBeNull();
		expect(fake.calls.filter((c) => c.cmd[0] === 'playlistcontrol' || c.cmd[0] === 'playlist')).toEqual([]);
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
		expect((screen.getByRole('button', { name: 'Play' }) as HTMLButtonElement).disabled).toBe(true);
		const menu = await openMenu(HEADER);
		const more = [...menu.querySelectorAll('[role=menuitem]')].find((i) => i.textContent?.trim() === 'More…')!;
		expect(more.getAttribute('aria-disabled')).toBe('true');
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

describe('moving playback in Play on… (it took over from the "Playing on" strip)', () => {
	test('in reach: moving playback to this device works', async () => {
		const r = render(PlayerSwitcher, { onclose: () => {} });
		const menu = await openMenu('More options for this device', r.container);
		const move = menu.querySelector('[role=menuitem]')!;
		expect(move.textContent).toContain('Move playback to this device');
		expect(move.getAttribute('aria-disabled')).toBeNull();
	});

	test('out of reach: moving playback is unavailable (it needs the server), says why, and does nothing', async () => {
		fake.app.reach = 'unreachable';
		const r = render(PlayerSwitcher, { onclose: () => {} });
		for (const trigger of ['More options for this device']) {
			const menu = await openMenu(trigger, r.container);
			const move = menu.querySelector<HTMLElement>('[role=menuitem]')!;
			expect(move.getAttribute('aria-disabled')).toBe('true');
			explained(move);
			await fireEvent.click(move);
			await tick();
		}
		expect(fake.app.playerId).toBe(LOUNGE);
		expect(fake.moves).toEqual([]);
	});
});
