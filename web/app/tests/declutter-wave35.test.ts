// Wave 3.5, phase 1 (docs/plans/wave3_5-declutter.md): the layout rules as assertions, and every
// action that moved still reachable. Page headers are Play · Shuffle · Download · ⋯ at most; a
// row has at most one trailing control (besides a drag handle); destructive actions only in a
// menu; one snackbar at a time; "Continue on…" is a snackbar with an action; wide Now playing
// has Queue and ⋯ and no second transport; empty states never claim "empty" when unread.
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import type { DownloadManager } from '#lib/offline/manager.svelte';

const params: Record<string, string | undefined> = { id: '7' };
vi.mock('$app/state', () => ({ page: { params, url: new URL('http://app.example/#/') } }));
vi.mock('#lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
vi.mock('#lib/offline/index.svelte', async () => {
	const { DownloadManager } = await import('#lib/offline/manager.svelte');
	const { MemoryBackend, OfflineStore } = await import('#lib/offline/store');
	const { HandoffOffer } = await import('#lib/offline/handoff.svelte');
	const { PlaySync } = await import('#lib/offline/plays.svelte');
	const offlineStore = new OfflineStore(new MemoryBackend());
	const hang = (() => new Promise(() => {})) as typeof fetch;
	const downloads = new DownloadManager({ store: offlineStore, authed: (fn) => fn('t'), fetch: hang, sleep: async () => {}, waitForWifi: () => false, capBytes: () => 0 });
	const plays = new PlaySync({ store: offlineStore, authed: (fn) => fn('t'), fetch: hang });
	// The offline player as the hand-off sees it: two downloaded tracks, the first playing.
	const key = (id: number) => String(id).padStart(64, '0');
	// Empty (not playing downloads) unless a test fills it.
	const offlinePlayer = {
		queue: [] as { id: number; key: string }[],
		index: 0,
		position: 12,
		playing: true,
		user: 'alice',
		get active() {
			return this.queue.length > 0;
		},
		stop: vi.fn(),
		forget: vi.fn(),
		key,
	};
	const handoff = new HandoffOffer();
	const lookupTrack = async (id: number) => ({ id, key: key(id) });
	return { offlineStore, downloads, plays, offlinePlayer, handoff, lookupTrack };
});
const fakeMe = {
	deleteStation: vi.fn(async () => {}),
	queueEdit: vi.fn(async () => {
		throw Object.assign(new Error('not offered'), { status: 404 });
	}),
};
vi.mock('#lib/state/me', () => ({ me: fakeMe, routine: { alarms: async () => ({ alarms: [] }) } }));

const fake = await import('./helpers/fake-app.svelte');
const offline = await import('#lib/offline/index.svelte');
const downloads = offline.downloads as DownloadManager;
const handoff = offline.handoff as InstanceType<(typeof import('#lib/offline/handoff.svelte'))['HandoffOffer']>;
const { device } = await import('#lib/player/device.svelte');
const { openMenu, choose, menuItems } = await import('./helpers/menu');
const { snackbars } = await import('#lib/components/snackbars.svelte');

const P = '02:00:00:00:00:01';
const KITCHEN = '02:00:00:00:00:02';
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
let width = 1300;
beforeAll(() => {
	vi.stubGlobal('IntersectionObserver', NoObserver);
	vi.stubGlobal('ResizeObserver', NoObserver);
	vi.stubGlobal('matchMedia', (q: string) => {
		const min = q.match(/min-width:\s*(\d+)px/);
		return { matches: min ? width >= Number(min[1]) : false, addEventListener() {}, removeEventListener() {} };
	});
});
afterAll(() => vi.unstubAllGlobals());

beforeEach(async () => {
	width = 1300;
	params.id = '7';
	fake.calls.length = 0;
	fake.moves.length = 0;
	fake.app.players = [
		{ playerid: P, name: 'Lounge' },
		{ playerid: KITCHEN, name: 'Kitchen' },
	];
	fake.app.playerId = P;
	fake.app.deviceMac = '';
	fake.app.isAdmin = false;
	fake.app.reach = 'online';
	fake.app.notice = '';
	fake.app.error = '';
	fake.app.now = { mode: 'play', time: 30, duration: 240, playlist_cur_index: 0, playlist_tracks: 2, 'mixer volume': 40, playlist_loop: [{ id: 11, title: 'One', artist: 'Band', album: 'Record', album_id: 7 }] };
	fake.handler.rpc = async (_p, c) => {
		if (c[0] === 'albums') return { albums_loop: [{ id: 7, album: 'Record', artist: 'Band', artwork_track_id: '11' }] };
		if (c[0] === 'titles') return { titles_loop: TRACKS };
		if (c[0] === 'playlists' && c[1] === 'tracks') return { playlisttracks_loop: TRACKS };
		if (c[0] === 'playlists') return { playlists_loop: [{ id: 7, playlist: 'Road trip' }] };
		if (c[0] === 'genres') return { genres_loop: [{ id: 7, genre: 'Jazz' }] };
		if (c[0] === 'status') return { playlist_loop: TRACKS, playlist_tracks: 2 };
		return {};
	};
	device.mac = '';
	await downloads.setUser(null);
	await downloads.setUser('alice', true);
});

/** The buttons of a page header's actions row (the download control's parts included). */
function headerButtons(c: HTMLElement): string[] {
	const row = c.querySelector('[data-page-actions]')!;
	return [...row.querySelectorAll<HTMLButtonElement>('button')].map((b) => (b.getAttribute('aria-label') ?? b.textContent ?? '').trim());
}

describe('rule 3: page headers are Play · Shuffle · Download · ⋯', () => {
	test('album: one row of four; Play next, Add to queue, Download as… and More… are in ⋯', async () => {
		const { default: Page } = await import('../src/routes/album/[id]/+page.svelte');
		const r = render(Page);
		await screen.findByRole('heading', { name: 'Record' });
		expect(headerButtons(r.container)).toEqual(['Play', 'Shuffle', 'Download', 'More options for Record']);
		expect(r.container.querySelector('[data-page-actions] select')).toBeNull();
		expect(await menuItems('More options for Record')).toEqual(['Play next', 'Add to queue', 'Download as…', 'More…']);
	});

	test('Shuffle turns shuffle on, then plays the album', async () => {
		const { default: Page } = await import('../src/routes/album/[id]/+page.svelte');
		render(Page);
		await screen.findByRole('heading', { name: 'Record' });
		fake.calls.length = 0;
		await fireEvent.click(screen.getByRole('button', { name: 'Shuffle' }));
		await waitFor(() =>
			expect(fake.calls.map((c) => c.cmd)).toEqual([
				['playlist', 'shuffle', 1],
				['playlistcontrol', 'cmd:load', 'album_id:7'],
			]),
		);
	});

	test('the moved actions still do what they did: Play next and Add to queue from ⋯', async () => {
		const { default: Page } = await import('../src/routes/album/[id]/+page.svelte');
		render(Page);
		await screen.findByRole('heading', { name: 'Record' });
		fake.calls.length = 0;
		await choose('More options for Record', 'Play next');
		await choose('More options for Record', 'Add to queue');
		expect(fake.calls.map((c) => c.cmd)).toEqual([
			['playlistcontrol', 'cmd:insert', 'album_id:7'],
			['playlistcontrol', 'cmd:add', 'album_id:7'],
		]);
		await choose('More options for Record', 'More…');
		expect(document.querySelector('dialog[aria-label="More for Record"]')).not.toBeNull();
	});

	test('artist: Play (not "Play all"), Shuffle, Download all albums, ⋯ with Add to queue and More…', async () => {
		const { default: Page } = await import('../src/routes/artist/[id]/[[role]]/+page.svelte');
		const r = render(Page);
		await waitFor(() => expect(r.container.querySelector('[data-page-actions]')).not.toBeNull());
		await waitFor(() => expect(headerButtons(r.container)).toEqual(['Play', 'Shuffle', 'Download all albums', 'More options for Band']));
		expect(await menuItems('More options for Band')).toEqual(['Add to queue', 'Download all as…', 'More…']);
	});

	test('playlist: admins get Rename and Delete in ⋯ (Delete last, confirmed in a dialog), never in the header', async () => {
		fake.app.isAdmin = true;
		const { default: Page } = await import('../src/routes/playlist/[id]/+page.svelte');
		const r = render(Page);
		await screen.findByRole('heading', { name: 'Road trip' });
		expect(headerButtons(r.container)).toEqual(['Play', 'Shuffle', 'Download', 'More options for Road trip']);
		expect(await menuItems('More options for Road trip')).toEqual(['Play next', 'Add to queue', 'Rename', 'Download as…', 'Delete playlist']);
		await fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
		await choose('More options for Road trip', 'Delete playlist');
		const dialog = await waitFor(() => document.querySelector('[data-confirm-dialog]')!);
		expect(dialog.textContent).toContain('Delete the playlist “Road trip”?');
		await fireEvent.click(dialog.querySelector('[data-confirm-cancel]')!);
		expect(fake.calls.some((c) => c.cmd[1] === 'delete')).toBe(false);
		await choose('More options for Road trip', 'Delete playlist');
		await fireEvent.click(document.querySelector('[data-confirm-ok]')!);
		await waitFor(() => expect(fake.calls.some((c) => c.cmd[0] === 'playlists' && c.cmd[1] === 'delete')).toBe(true));
	});

	test('playlist rows (admins): one ⋯ with Move up, Move down and Remove from the playlist', async () => {
		fake.app.isAdmin = true;
		const { default: Page } = await import('../src/routes/playlist/[id]/+page.svelte');
		const r = render(Page);
		await screen.findByRole('heading', { name: 'Road trip' });
		for (const li of r.container.querySelectorAll('ol.q > li')) {
			const trailing = [...li.children].filter((el) => el.matches('button, .rowmenu') && !el.classList.contains('main'));
			expect(trailing).toHaveLength(1);
		}
		fake.calls.length = 0;
		await choose('More options for Two', 'Move up', r.container);
		await waitFor(() => expect(fake.calls.map((c) => c.cmd)).toContainEqual(['playlists', 'edit', 'playlist_id:7', 'cmd:move', 'index:1', 'toindex:0']));
	});

	test('genre and year: Play, Shuffle and ⋯ (Add to queue, More…); genre’s tabs are the shared chips', async () => {
		const { default: Genre } = await import('../src/routes/genre/[id]/+page.svelte');
		const g = render(Genre);
		await waitFor(() => expect(g.container.querySelector('h1')!.textContent).toBe('Jazz'));
		expect(headerButtons(g.container)).toEqual(['Play', 'Shuffle', 'More options for Jazz']);
		expect(g.getByRole('tablist', { name: 'Show' }).querySelectorAll('[role=tab]')).toHaveLength(2);
		g.unmount();
		params.year = '1999';
		const { default: Year } = await import('../src/routes/year/[year]/+page.svelte');
		const y = render(Year);
		expect(headerButtons(y.container)).toEqual(['Play', 'Shuffle', 'More options for 1999']);
		expect(await menuItems('More options for 1999', y.container)).toEqual(['Add to queue', 'More…']);
	});
});

describe('rule 3: a row has at most one trailing control', () => {
	test('the queue: the drag handle, the track, and its ⋯; no ✕, no move buttons, no gapless marks', async () => {
		const { default: Queue } = await import('../src/routes/queue/+page.svelte');
		const r = render(Queue);
		await waitFor(() => expect(r.container.querySelectorAll('li[data-index]').length).toBe(2));
		for (const li of r.container.querySelectorAll('li[data-index]')) {
			const controls = [...li.querySelectorAll(':scope > button, :scope > .rowmenu')];
			expect(controls.map((el) => (el.classList.contains('main') ? 'main' : 'menu'))).toEqual(['main', 'menu']);
			expect(li.querySelector('[data-gapless]')).toBeNull();
		}
		// The header: the title and ⋯, with Clear the queue (confirmed) inside.
		expect(r.container.querySelectorAll('header button').length).toBe(1);
		expect(await menuItems('Queue options', r.container)).toEqual(['Clear upcoming…', 'Clear the queue…']);
		await fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
		await choose('Queue options', 'Clear the queue…', r.container);
		const dialog = await waitFor(() => document.querySelector('[data-confirm-dialog]')!);
		expect(dialog.textContent).toContain('Clear the queue?');
		await fireEvent.click(dialog.querySelector('[data-confirm-ok]')!);
		await waitFor(() => expect(fake.calls.map((c) => c.cmd)).toContainEqual(['playlist', 'clear']));
	});

	test('Home’s station cards: no bin, a ⋯ with Delete station, confirmed', async () => {
		const { default: Cards } = await import('#lib/components/StationCards.svelte');
		const onremoved = vi.fn();
		const r = render(Cards, { stations: [{ id: 4, name: 'Fresh', kind: 'fresh', spec: { kind: 'fresh' }, broken: null }], removable: true, onremoved });
		expect(r.queryByRole('button', { name: /^Delete/ })).toBeNull();
		await choose('More options for Fresh', 'Delete station', r.container);
		await fireEvent.click(await waitFor(() => document.querySelector<HTMLElement>('[data-confirm-ok]')!));
		await waitFor(() => expect(onremoved).toHaveBeenCalledWith(4));
		expect(fakeMe.deleteStation).toHaveBeenCalledWith(4);
	});
});

describe('rule 1: one message at a time', () => {
	test('two snackbars with messages: only the newest shows; when it goes, the older one shows again', async () => {
		const { default: Snackbar } = await import('#lib/components/Snackbar.svelte');
		const a = render(Snackbar, { message: 'First', ondismiss: () => {} });
		const b = render(Snackbar, { message: '', ondismiss: () => {} });
		await tick();
		const shown = () => [...document.querySelectorAll('[data-snackbar][data-shown]')].map((s) => s.textContent?.trim().split('\n')[0]);
		expect(shown()).toEqual(['First']);
		await b.rerender({ message: 'Second', ondismiss: () => {} });
		await tick();
		expect(document.querySelectorAll('[data-snackbar][data-shown]')).toHaveLength(1);
		expect(document.querySelector('[data-snackbar][data-shown] .msg')!.textContent).toBe('Second');
		await b.rerender({ message: '', ondismiss: () => {} });
		await tick();
		expect(document.querySelector('[data-snackbar][data-shown] .msg')!.textContent).toBe('First');
		a.unmount();
		b.unmount();
		expect(snackbars.count).toBe(0);
	});

	test('a snackbar’s words are in its live region even while another shows (screen readers hear both)', async () => {
		const { default: Snackbar } = await import('#lib/components/Snackbar.svelte');
		const a = render(Snackbar, { message: 'Saved', ondismiss: () => {} });
		const b = render(Snackbar, { message: 'Removed', ondismiss: () => {} });
		await tick();
		expect([...document.querySelectorAll('[data-snackbar] [role=status]')].map((s) => s.textContent)).toEqual(['Saved', 'Removed']);
		a.unmount();
		b.unmount();
	});
});

describe('Continue on… (a snackbar with an action)', () => {
	const player = offline.offlinePlayer as unknown as { queue: { id: number; key: string }[]; key: (id: number) => string };
	beforeEach(() => {
		player.queue = [5, 6].map((id) => ({ id, key: player.key(id) }));
		handoff.offered = 'alice';
		handoff.busy = '';
	});
	afterEach(() => {
		handoff.dismiss();
		player.queue = [];
	});

	test('offers the player last used, with Continue and Other player…; Not now (dismiss) withdraws it', async () => {
		const { default: ContinueOn } = await import('#lib/components/ContinueOn.svelte');
		const r = render(ContinueOn);
		await tick();
		const snack = r.container.querySelector('[data-snackbar][data-shown]')!;
		expect(snack.textContent).toContain('Back online. Continue 2 tracks from your downloads on Lounge?');
		expect(within(snack as HTMLElement).getByRole('button', { name: 'Continue' })).toBeTruthy();
		expect(within(snack as HTMLElement).getByRole('button', { name: 'Other player…' })).toBeTruthy();
		await fireEvent.click(within(snack as HTMLElement).getByRole('button', { name: 'Dismiss' }));
		expect(handoff.offered).toBeNull();
	});

	test('Other player… lists the players this user may control; choosing one continues there', async () => {
		const { default: ContinueOn } = await import('#lib/components/ContinueOn.svelte');
		const r = render(ContinueOn);
		await tick();
		await fireEvent.click(r.getByRole('button', { name: 'Other player…' }));
		const list = await waitFor(() => document.querySelector('[data-continue-choices]')!);
		expect([...list.querySelectorAll('button')].map((b) => b.textContent?.trim())).toEqual(['Lounge', 'Kitchen']);
		await fireEvent.click([...list.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Kitchen')!);
		await waitFor(() => expect(fake.calls.some((c) => c.player === KITCHEN && c.cmd[0] === 'playlistcontrol')).toBe(true));
		await waitFor(() => expect(fake.app.playerId).toBe(KITCHEN));
	});

	test('nothing offered: nothing shown', async () => {
		handoff.dismiss();
		const { default: ContinueOn } = await import('#lib/components/ContinueOn.svelte');
		const r = render(ContinueOn);
		await tick();
		expect(r.container.querySelector('[data-snackbar][data-shown]')).toBeNull();
	});
});

describe('wide Now playing: Queue and ⋯, no second transport', () => {
	test('the action row is exactly Queue and ⋯; ⋯ holds Sleep timer, This track, Signal path and Car mode', async () => {
		const { default: NowPlaying } = await import('../src/routes/+page.svelte');
		const r = render(NowPlaying);
		await tick();
		const bar = r.container.querySelector('.np .bar')!;
		expect([...bar.querySelectorAll(':scope > button, :scope > .rowmenu > button')].map((b) => b.getAttribute('aria-label'))).toEqual(['Show the queue beside Now playing', 'More options']);
		expect(await menuItems('More options', r.container)).toEqual(['Sleep timer', 'This track', 'Signal path', 'Car mode']);
		// No Lyrics button or dialog: the Lyrics layout is the one way in (and the L key).
		expect(r.container.querySelector('[data-lyrics-button]')).toBeNull();
		// The player's name by the volume, as the Play on button (the mini player is hidden here).
		expect(r.container.querySelector('.volrow [data-output-button]')!.textContent).toContain('Lounge');
	});

	test('the ⋯ items open what the buttons did', async () => {
		const { default: NowPlaying } = await import('../src/routes/+page.svelte');
		const r = render(NowPlaying);
		await tick();
		await choose('More options', 'Sleep timer', r.container);
		await waitFor(() => expect(r.container.querySelector('[data-sleep-sheet]')).not.toBeNull());
		document.querySelector<HTMLDialogElement>('dialog[open]')?.close();
		await choose('More options', 'Signal path', r.container);
		await waitFor(() => expect(document.querySelector('dialog[open] [data-signal-path]')).not.toBeNull());
	});

	test('a running sleep timer: its time left is shown, and ⋯ can turn it off', async () => {
		fake.app.now = { ...fake.app.now!, will_sleep_in: 600 };
		const { default: NowPlaying } = await import('../src/routes/+page.svelte');
		const r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('[data-sleep-left]')!.textContent).toContain('10 min');
		await choose('More options', 'Turn the sleep timer off', r.container);
		expect(fake.app.sleepCalls.at(-1)).toEqual({ choice: 'off', scope: null });
	});

	test('no player chosen: one wording, and a button that opens Play on…', async () => {
		fake.app.playerId = '';
		fake.app.now = null;
		const { default: NowPlaying } = await import('../src/routes/+page.svelte');
		const r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('h1')!.textContent).toBe('No player chosen');
		await fireEvent.click(r.getByRole('button', { name: 'Choose a player' }));
		expect(r.container.querySelector('[role=dialog][aria-label="Play on"]')).not.toBeNull();
	});
});

describe('empty states never lie', () => {
	test('the queue, never read and the server out of reach: it can’t be loaded, not "empty"', async () => {
		fake.app.reach = 'unreachable';
		fake.handler.rpc = async () => {
			throw new TypeError('Failed to fetch');
		};
		const { default: Queue } = await import('../src/routes/queue/+page.svelte');
		const r = render(Queue);
		await tick();
		await new Promise((res) => setTimeout(res, 0));
		expect(r.container.textContent).not.toContain('The queue is empty');
		expect(r.container.querySelector('[data-queue-unloaded]')!.textContent).toBe('The queue can’t be loaded until your music server is back.');
		r.unmount();
		const { default: QueueList } = await import('#lib/components/QueueList.svelte');
		const peek = render(QueueList);
		await tick();
		expect(peek.container.textContent).not.toContain('The queue is empty');
		expect(peek.container.querySelector('[data-queue-unloaded]')).not.toBeNull();
	});

	test('read and really empty: it says so', async () => {
		fake.handler.rpc = async () => ({ playlist_loop: [], playlist_tracks: 0 });
		const { default: Queue } = await import('../src/routes/queue/+page.svelte');
		const r = render(Queue);
		await waitFor(() => expect(r.container.textContent).toContain('The queue is empty.'));
		expect(r.container.querySelector('[data-queue-unloaded]')).toBeNull();
	});
});

describe('rule 2: one mark per meaning', () => {
	test('a downloaded track has the one "available offline" mark; ✕ is never a mark', async () => {
		const { default: TrackList } = await import('#lib/components/TrackList.svelte');
		const r = render(TrackList, {
			tracks: TRACKS,
			onplay: () => {},
			badge: (t: { id?: unknown }) => (t.id === 11 ? { kind: 'done' as const, label: 'Downloaded' } : { kind: 'failed' as const, label: 'Download failed' }),
		});
		const done = r.container.querySelector('[data-track-mark="done"] svg path')!.getAttribute('d');
		const { default: Icon } = await import('#lib/components/Icon.svelte');
		const icon = render(Icon, { name: 'downloaded' });
		expect(done).toBe(icon.container.querySelector('path')!.getAttribute('d'));
		const close = render(Icon, { name: 'close' });
		const cross = close.container.querySelector('path')!.getAttribute('d');
		for (const p of r.container.querySelectorAll('[data-track-mark] path')) expect(p.getAttribute('d')).not.toBe(cross);
	});
});

describe('Home', () => {
	test('Edit Home is a quiet text button at the end, not beside the gear', async () => {
		const src = (await import('../src/routes/home/+page.svelte?raw')).default as string;
		const header = src.slice(src.indexOf('<header>'), src.indexOf('</header>'));
		expect(header).not.toContain('Edit Home');
		expect(src).toMatch(/class="link-btn edit-home"[^>]*data-edit-home>Edit Home<\/button>/);
	});
});

describe('the playing-on strip is gone (only "Moving…" remains)', () => {
	test('a menu of the player picker reaches what the strip offered', async () => {
		fake.app.deviceMac = '02:4c:41:00:00:01';
		fake.app.players = [...fake.app.players, { playerid: '02:4c:41:00:00:01', name: 'This browser' }];
		vi.stubGlobal('AudioWorkletNode', class {});
		vi.stubGlobal('Worker', class {});
		device.state = 'connected';
		device.mac = '02:4c:41:00:00:01';
		const { default: PlayerSwitcher } = await import('#lib/components/PlayerSwitcher.svelte');
		const r = render(PlayerSwitcher, { onclose: () => {} });
		const menu = await openMenu('More options for this device', r.container);
		expect(menu.textContent).toContain('Move playback to this device');
		device.state = 'off';
		device.mac = '';
	});
});
