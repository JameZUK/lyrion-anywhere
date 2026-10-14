// Wave 3.5 item 14: downloads play through the main mini player, Now playing and Queue (two
// engines, one face). While the downloads engine's queue is active, the transport acts on it,
// the screens show its track, position and queue, and LMS-only controls are left out.
import { readFileSync } from 'node:fs';
import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
vi.mock('$lib/offline/index.svelte', async () => {
	const { DownloadManager } = await import('$lib/offline/manager.svelte');
	const { MemoryBackend, OfflineStore } = await import('$lib/offline/store');
	const { HandoffOffer } = await import('$lib/offline/handoff.svelte');
	const { PlaySync } = await import('$lib/offline/plays.svelte');
	const offlineStore = new OfflineStore(new MemoryBackend());
	const hang = (() => new Promise(() => {})) as typeof fetch;
	const downloads = new DownloadManager({ store: offlineStore, authed: (fn) => fn('t'), fetch: hang, sleep: async () => {}, waitForWifi: () => false, capBytes: () => 0 });
	const plays = new PlaySync({ store: offlineStore, authed: (fn) => fn('t'), fetch: hang });
	const { FakeOffline } = await import('./helpers/fake-offline.svelte');
	const offlinePlayer = new FakeOffline();
	return { offlineStore, downloads, plays, offlinePlayer, handoff: new HandoffOffer(), lookupTrack: async () => null };
});

const fake = await import('./helpers/fake-app.svelte');
const offline = await import('$lib/offline/index.svelte');
const engine = offline.offlinePlayer as unknown as {
	queue: unknown[];
	index: number;
	playing: boolean;
	position: number;
	duration: number;
	cover: string;
	calls: string[];
};
const { controls } = await import('$lib/player/controls.svelte');
const { now } = await import('$lib/player/now.svelte');
const { default: PlayerBar } = await import('$lib/components/PlayerBar.svelte');
const { default: NowPlaying } = await import('../src/routes/+page.svelte');
const { default: QueuePage } = await import('../src/routes/queue/+page.svelte');
const { default: QueueList } = await import('$lib/components/QueueList.svelte');

const P = '02:00:00:00:00:01';
const stored = (id: number, title: string) => ({ user: 'alice', key: String(id).padStart(64, '0'), id, title, artist: 'The Downloads', album: 'Kept Here', albumId: 9, tracknum: id, duration: 200, format: 'opus', codec: 'opus', ext: 'opus', size: 1, contentType: 'audio/ogg', downloadedAt: 1, chunks: 1 });

class NoObserver {
	observe() {}
	unobserve() {}
	disconnect() {}
	takeRecords() {
		return [];
	}
}
let width = 360;
beforeAll(() => {
	vi.stubGlobal('IntersectionObserver', NoObserver);
	vi.stubGlobal('ResizeObserver', NoObserver);
	vi.stubGlobal('matchMedia', (q: string) => {
		const min = q.match(/min-width:\s*(\d+)px/);
		return { matches: min ? width >= Number(min[1]) : false, addEventListener() {}, removeEventListener() {} };
	});
	HTMLCanvasElement.prototype.getContext = (() => null) as typeof HTMLCanvasElement.prototype.getContext;
	// Tracks cross-fade on Now playing (jsdom has no Web Animations).
	Element.prototype.animate ??= function () {
		return { cancel() {}, finished: Promise.resolve(), onfinish: null } as unknown as Animation;
	};
});
afterAll(() => vi.unstubAllGlobals());

beforeEach(() => {
	width = 360;
	fake.calls.length = 0;
	fake.app.players = [{ playerid: P, name: 'Lounge' }];
	fake.app.playerId = P;
	fake.app.reach = 'online';
	fake.app.now = { mode: 'play', time: 30, duration: 240, playlist_cur_index: 0, playlist_tracks: 3, playlist_loop: [{ id: 1, title: 'On the Lounge', artist: 'LMS Band', album: 'Server Album' }] };
	engine.queue = [stored(21, 'First kept'), stored(22, 'Second kept'), stored(23, 'Third kept')];
	engine.index = 0;
	engine.playing = true;
	engine.position = 42;
	engine.duration = 200;
	engine.calls = [];
});
afterEach(() => {
	engine.queue = [];
});

describe('the transport while downloads play', () => {
	test('acts on the downloads engine, never on the selected player', async () => {
		expect(controls.downloads).toBe(true);
		expect(controls.playing).toBe(true);
		controls.toggle();
		controls.next();
		controls.previous();
		await controls.seek(17);
		controls.jump(2);
		expect(engine.calls).toEqual(['toggle', 'next', 'previous', 'seek 17', 'jump 2']);
		expect(fake.calls.filter((c) => c.cmd[0] !== 'status')).toEqual([]);
	});

	test('what works: the transport and seeking; nothing of LMS (volume, shuffle, repeat, sleep)', () => {
		const g = controls.gates;
		expect([g.playPause, g.previous, g.next, g.seek, g.volume, g.other]).toEqual([true, true, true, true, false, false]);
		expect(g.jump(1)).toBe(true);
		expect(g.jump(3)).toBe(false);
		engine.index = 2;
		expect(controls.gates.next).toBe(false);
	});

	test('the view: the download’s track, position and queue', () => {
		expect(now.downloads).toBe(true);
		expect(now.track?.title).toBe('First kept');
		expect(now.elapsed).toBe(42);
		expect(now.duration).toBe(200);
		expect(now.nextDownload?.title).toBe('Second kept');
		expect(now.queue.map((t) => t.title)).toEqual(['First kept', 'Second kept', 'Third kept']);
		engine.queue = [];
		expect(now.downloads).toBe(false);
		expect(now.track?.title).toBe('On the Lounge');
	});
});

describe('one mini player', () => {
	test('shows the download, "Downloads · artist", and no player name; its buttons play the downloads', async () => {
		const r = render(PlayerBar);
		await tick();
		expect(r.container.querySelector('.title')!.textContent).toBe('First kept');
		expect(r.container.querySelector('.sub')!.textContent).toContain('Downloads · The Downloads');
		expect(r.container.querySelector('[data-mini-on]')).toBeNull();
		await fireEvent.click(r.getByRole('button', { name: 'Pause' }));
		await fireEvent.click(r.getByRole('button', { name: 'Next' }));
		expect(engine.calls).toEqual(['toggle', 'next']);
		// Play on… is still there.
		expect(r.container.querySelector('[data-output-button]')).not.toBeNull();
	});

	test('the layout has no separate downloads bar any more', () => {
		const layout = readFileSync(`${__dirname}/../src/routes/+layout.svelte`, 'utf8');
		expect(layout).not.toMatch(/OfflineBar/);
		expect(layout).toMatch(/<div class="barwrap" class:on-np=\{path === '\/'\}><PlayerBar \/><\/div>/);
	});
});

describe('one Now playing', () => {
	test('phones: the download, no rating or quality badge; ⋮ has Stop playing downloads and nothing of LMS', async () => {
		const r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('h1.title')!.textContent).toBe('First kept');
		expect(r.container.querySelector('[data-rating-toggle], [data-quality-badge]')).toBeNull();
		// Up next is the next download; the queue opens on a tap.
		expect(r.container.querySelector('[data-up-next]')!.textContent).toContain('Second kept');
		await fireEvent.click(r.getByRole('button', { name: 'More options' }));
		const sheet = document.querySelector('dialog[open] [data-more-sheet]')!;
		expect(sheet.querySelector('[data-track-menu], [data-sleep-button], [data-car-link], [data-visualiser-style-row]')).toBeNull();
		await fireEvent.click(sheet.querySelector('[data-stop-downloads]')!);
		expect(engine.calls).toContain('stop');
		r.unmount();
	});

	test('wide windows: ⋯ is Stop playing downloads only', async () => {
		width = 1300;
		const r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('h1.title')!.textContent).toBe('First kept');
		expect(r.container.textContent).toContain('1 of 3 · Downloads');
		await fireEvent.click(r.getByRole('button', { name: 'More options' }));
		expect([...r.container.querySelectorAll('[role=menuitem]')].map((m) => m.textContent?.trim())).toEqual(['Stop playing downloads']);
		r.unmount();
	});
});

describe('one Queue', () => {
	test('the Queue page lists the downloads in play order; a tap plays from there; nothing to edit', async () => {
		const r = render(QueuePage);
		await tick();
		const list = r.container.querySelector('[data-downloads-queue]')!;
		expect([...list.querySelectorAll('.title')].map((t) => t.textContent)).toEqual(['First kept', 'Second kept', 'Third kept']);
		expect(list.querySelector('[aria-current="true"]')!.textContent).toContain('First kept');
		expect(r.container.textContent).toContain('Downloads · 3 tracks');
		expect(r.container.querySelector('[data-row-more], .handle, [data-queue-more]')).toBeNull();
		await fireEvent.click(r.getByRole('button', { name: 'Play Third kept by The Downloads. Downloaded' }));
		expect(engine.calls).toEqual(['jump 2']);
		// The selected player's queue isn't read meanwhile.
		expect(fake.calls.some((c) => c.cmd[0] === 'status' && c.cmd[1] !== '-')).toBe(false);
		r.unmount();
	});

	test('the queue sheet shows them too', async () => {
		const onplay = vi.fn();
		const r = render(QueueList, { onplay });
		await tick();
		await fireEvent.click(r.getByRole('button', { name: /^Play Second kept/ }));
		expect(engine.calls).toEqual(['jump 1']);
		expect(onplay).toHaveBeenCalled();
		r.unmount();
	});

	test('hostile titles stay text', async () => {
		engine.queue = [stored(31, '<img src=x onerror=alert(1)>')];
		const r = render(QueuePage);
		await waitFor(() => expect(r.container.textContent).toContain('<img src=x onerror=alert(1)>'));
		expect(r.container.querySelector('img[src="x"]')).toBeNull();
		r.unmount();
	});
});
