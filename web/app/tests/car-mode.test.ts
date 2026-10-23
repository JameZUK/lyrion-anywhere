// Car mode (wave2-design.md §5.1): giant previous / play-pause / next (holding play stops),
// Start radio from the current track, a large elapsed time, one level of Favourites and
// Stations, an obvious Exit, no visualiser, and the screen kept on only while playing.
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { goto } from '$app/navigation';
import type { Station } from '#lib/api/me';
import { ScreenWake, type WakeLockApi, type WakeSentinel } from '#lib/player/wake-lock';

vi.mock('#lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const stations: Station[] = [
	{ id: 4, name: 'Fresh finds', kind: 'fresh', spec: { kind: 'fresh' }, broken: null },
	{ id: 5, name: 'Broken one', kind: 'menu', spec: { kind: 'fresh' }, broken: 'gone' },
];
vi.mock('#lib/state/me', () => ({ me: { stations: async () => structuredClone(stations) } }));
const started: number[] = [];
vi.mock('#lib/stations/active.svelte', () => ({
	activeStation: {
		error: '',
		start: async (s: Station) => (started.push(s.id), true),
	},
}));
const fake = await import('./helpers/fake-app.svelte');
const { DSTM_PREF } = await import('#lib/lms/discovery');

const PLAYER = '02:00:00:00:00:01';

/** A fake Screen Wake Lock API that records what is held. */
class FakeWake implements WakeLockApi {
	requests = 0;
	live: (WakeSentinel & { released: boolean })[] = [];
	refuse = false;
	async request(type: 'screen') {
		expect(type).toBe('screen');
		this.requests++;
		if (this.refuse) throw new Error('NotAllowedError');
		const fns: (() => void)[] = [];
		const s = {
			released: false,
			async release() {
				if (s.released) return;
				s.released = true;
				for (const f of fns) f();
			},
			addEventListener: (_t: 'release', f: () => void) => fns.push(f),
		};
		this.live.push(s);
		return s;
	}
	get held() {
		return this.live.filter((s) => !s.released).length;
	}
}
let wake: FakeWake;

function setVisibility(v: 'visible' | 'hidden') {
	Object.defineProperty(document, 'visibilityState', { value: v, configurable: true });
	document.dispatchEvent(new Event('visibilitychange'));
}

const { default: Car } = await import('../src/routes/car/+page.svelte');

beforeEach(() => {
	wake = new FakeWake();
	Object.defineProperty(navigator, 'wakeLock', { value: wake, configurable: true });
	setVisibility('visible');
	started.length = 0;
	fake.calls.length = 0;
	fake.app.players = [{ playerid: PLAYER, name: 'Test player' }];
	fake.app.playerId = PLAYER;
	fake.app.elapsed = 151;
	fake.app.now = { mode: 'play', time: 151, duration: 245, playlist_loop: [{ id: 9, title: 'A long drive', artist: 'Example Band', album: 'Roads', duration: 245 }] };
	fake.handler.rpc = async () => ({});
	vi.mocked(goto).mockClear();
});
afterEach(() => {
	vi.useRealTimers();
	delete (navigator as { wakeLock?: unknown }).wakeLock;
});

const sent = () => fake.calls.map((c) => c.cmd);
const play = () => screen.getByRole('button', { name: /^(Play|Pause)$/ });

describe('the driving screen', () => {
	test('the track, a large elapsed time, and every control named', () => {
		render(Car);
		expect(screen.getByRole('heading', { name: 'Car mode' })).toBeTruthy();
		expect(screen.getByText('A long drive')).toBeTruthy();
		expect(screen.getByText('Example Band')).toBeTruthy();
		expect(screen.getByLabelText('Elapsed 2:31 of 4:05')).toBeTruthy();
		for (const name of ['Exit', 'Previous', 'Pause', 'Next', 'Start radio', 'Favourites', 'Stations']) expect(screen.getByRole('button', { name })).toBeTruthy();
		// Holding play/pause is explained to everyone.
		expect(play().getAttribute('aria-describedby')).toBe('car-hold');
		expect(document.getElementById('car-hold')!.textContent).toMatch(/Hold play to stop/);
	});

	test('no visualiser', () => {
		const r = render(Car);
		expect(r.container.querySelector('[data-visualiser], canvas')).toBeNull();
	});

	test('previous, play/pause and next control the player', async () => {
		render(Car);
		await fireEvent.click(screen.getByRole('button', { name: 'Previous' }));
		await fireEvent.click(play());
		await fireEvent.click(screen.getByRole('button', { name: 'Next' }));
		expect(sent()).toEqual([
			['playlist', 'index', '-1'],
			['pause', 1],
			['playlist', 'index', '+1'],
		]);
		expect(fake.calls.every((c) => c.player === PLAYER)).toBe(true);
	});

	test('holding play/pause stops (and the click that follows does nothing more)', async () => {
		vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
		render(Car);
		await fireEvent.pointerDown(play(), { pointerType: 'touch' });
		await vi.advanceTimersByTimeAsync(750);
		await fireEvent.pointerUp(play(), { pointerType: 'touch' });
		await fireEvent.click(play());
		expect(sent()).toEqual([['stop']]);
		expect(screen.getByRole('status').textContent).toBe('Stopped');
	});

	test('a short press only plays or pauses', async () => {
		vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
		render(Car);
		await fireEvent.pointerDown(play(), { pointerType: 'touch' });
		await vi.advanceTimersByTimeAsync(200);
		await fireEvent.pointerUp(play(), { pointerType: 'touch' });
		await fireEvent.click(play());
		await vi.advanceTimersByTimeAsync(1000);
		expect(sent()).toEqual([['pause', 1]]);
	});

	test('the context-menu key (or Shift+F10) stops, for keyboards', async () => {
		render(Car);
		await fireEvent.contextMenu(play());
		expect(sent()).toEqual([['stop']]);
	});

	test('Exit, or Escape, goes back to Now playing', async () => {
		render(Car);
		await fireEvent.click(screen.getByRole('button', { name: 'Exit' }));
		expect(goto).toHaveBeenCalledWith('#/');
		vi.mocked(goto).mockClear();
		await fireEvent.keyDown(document.body, { key: 'Escape' });
		expect(goto).toHaveBeenCalledWith('#/');
	});

	test('without a player or track the transport is disabled', () => {
		fake.app.now = null;
		render(Car);
		for (const name of ['Previous', 'Play', 'Next', 'Start radio']) expect((screen.getByRole('button', { name }) as HTMLButtonElement).disabled).toBe(true);
		expect((screen.getByRole('button', { name: 'Exit' }) as HTMLButtonElement).disabled).toBe(false);
	});
});

describe('Start radio', () => {
	test('from the current track, through the server’s own offers', async () => {
		fake.handler.rpc = async (_p, c) => {
			if (c[0] === 'trackinfo') return { item_loop: [{ text: 'Instant mix', nextWindow: 'nowPlaying', actions: { go: { cmd: ['audiomuseai', 'mix'], params: { track_id: 9 } } } }] };
			if (c[0] === 'dontstopthemusicsetting') return { item_loop: [{ text: 'Similar', radio: 1, actions: { do: { cmd: ['playerpref', DSTM_PREF, 'PLUGIN_DSTM_SIMILAR'] } } }] };
			return {};
		};
		render(Car);
		await fireEvent.click(screen.getByRole('button', { name: 'Start radio' }));
		await waitFor(() => expect(screen.getByRole('status').textContent).toBe('Radio started'));
		expect(sent().at(-1)).toEqual(['audiomuseai', 'mix', 'track_id:9']);
	});

	test('says so when the server has no radio', async () => {
		render(Car);
		await fireEvent.click(screen.getByRole('button', { name: 'Start radio' }));
		await waitFor(() => expect(screen.getByRole('status').textContent).toMatch(/AudioMuse-AI or Don’t Stop The Music/));
	});

	test('not for a stream', () => {
		fake.app.now = { mode: 'play', time: 5, playlist_loop: [{ id: -3, title: 'Radio stream', remote: 1 }] };
		render(Car);
		expect((screen.getByRole('button', { name: 'Start radio' }) as HTMLButtonElement).disabled).toBe(true);
	});
});

describe('one level of Favourites and Stations', () => {
	test('favourites that play at once, and nothing deeper', async () => {
		fake.handler.rpc = async (_p, c) =>
			c[0] === 'favorites'
				? {
						loop_loop: [
							{ id: 'aa.0', name: 'Morning radio', isaudio: 1 },
							{ id: 'aa.1', name: 'A folder', isaudio: 0, hasitems: 1 },
						],
					}
				: {};
		render(Car);
		const button = screen.getByRole('button', { name: 'Favourites' });
		await fireEvent.click(button);
		expect(button.getAttribute('aria-pressed')).toBe('true');
		const item = await screen.findByRole('button', { name: 'Morning radio' });
		expect(screen.queryByRole('button', { name: 'A folder' })).toBeNull();
		await fireEvent.click(item);
		expect(sent().at(-1)).toEqual(['favorites', 'playlist', 'play', 'item_id:aa.0']);
		// Back to the driving screen.
		expect(screen.getByText('A long drive')).toBeTruthy();
	});

	test('stations (not broken ones) start from a tap', async () => {
		render(Car);
		await fireEvent.click(screen.getByRole('button', { name: 'Stations' }));
		await fireEvent.click(await screen.findByRole('button', { name: 'Fresh finds' }));
		expect(screen.queryByRole('button', { name: 'Broken one' })).toBeNull();
		await waitFor(() => expect(started).toEqual([4]));
	});

	test('Escape closes the list before leaving car mode', async () => {
		render(Car);
		await fireEvent.click(screen.getByRole('button', { name: 'Stations' }));
		await screen.findByRole('region', { name: 'Stations' });
		await fireEvent.keyDown(document.body, { key: 'Escape' });
		expect(screen.queryByRole('region', { name: 'Stations' })).toBeNull();
		expect(goto).not.toHaveBeenCalled();
	});
});

describe('keeping the screen on', () => {
	test('held while playing; released on pause, when hidden, and on leaving', async () => {
		const r = render(Car);
		await waitFor(() => expect(wake.held).toBe(1));
		fake.app.now = { ...fake.app.now!, mode: 'pause' };
		await waitFor(() => expect(wake.held).toBe(0));
		fake.app.now = { ...fake.app.now!, mode: 'play' };
		await waitFor(() => expect(wake.held).toBe(1));
		setVisibility('hidden');
		await waitFor(() => expect(wake.held).toBe(0));
		setVisibility('visible');
		await waitFor(() => expect(wake.held).toBe(1));
		r.unmount();
		await waitFor(() => expect(wake.held).toBe(0));
		// Gone: becoming visible again doesn't ask any more.
		const asked = wake.requests;
		setVisibility('hidden');
		setVisibility('visible');
		await new Promise((r) => setTimeout(r, 10));
		expect(wake.requests).toBe(asked);
	});

	test('not held while paused', async () => {
		fake.app.now = { ...fake.app.now!, mode: 'pause' };
		render(Car);
		await new Promise((r) => setTimeout(r, 10));
		expect(wake.requests).toBe(0);
	});

	test('a refused or missing API is harmless', async () => {
		wake.refuse = true;
		render(Car);
		await waitFor(() => expect(wake.requests).toBe(1));
		expect(wake.held).toBe(0);
		delete (navigator as { wakeLock?: unknown }).wakeLock;
		expect(() => render(Car)).not.toThrow();
	});

	test('a pause while the lock is being asked for gives it straight back', async () => {
		let resolve: (s: WakeSentinel) => void = () => {};
		const slow: WakeLockApi = { request: () => new Promise((r) => (resolve = r)) };
		const w = new ScreenWake(slow, document);
		w.set(true);
		w.set(false);
		const s = { released: false, release: vi.fn(async () => void (s.released = true)) };
		resolve(s);
		await waitFor(() => expect(s.release).toHaveBeenCalled());
		expect(w.held).toBe(false);
		w.destroy();
	});
});
