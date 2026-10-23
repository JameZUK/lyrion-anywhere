// Haptics on the web (wave2-design.md §5.4): `navigator.vibrate` where it exists, off by
// default, one switch in Settings; light ticks on skips, scrub detents, volume steps, drags,
// ratings and the sleep timer. Without the API nothing happens (and nothing breaks).
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';

vi.mock('#lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const rated: [number, number][] = [];
let rateOk = true;
vi.mock('#lib/state/ratings', () => ({
	ratings: {
		style: 'stars',
		get: () => null,
		load: async () => {},
		rate: async (t: number, v: number) => (rated.push([t, v]), rateOk),
	},
}));
const fake = await import('./helpers/fake-app.svelte');
const { haptics, HAPTICS_KEY, PATTERNS, crossed } = await import('#lib/haptics/haptics.svelte');
const { default: SeekBar } = await import('#lib/components/SeekBar.svelte');
const { default: RatingControl } = await import('#lib/components/RatingControl.svelte');
const { default: SleepSheet } = await import('#lib/components/SleepSheet.svelte');

beforeAll(() => {
	vi.stubGlobal(
		'ResizeObserver',
		class {
			observe() {}
			unobserve() {}
			disconnect() {}
		},
	);
});

const buzz = vi.fn((_p: number | number[]) => true);
function withVibrate(on: boolean) {
	if (on) Object.defineProperty(navigator, 'vibrate', { value: buzz, configurable: true, writable: true });
	else delete (navigator as { vibrate?: unknown }).vibrate;
}

beforeEach(() => {
	localStorage.removeItem(HAPTICS_KEY);
	haptics.reload();
	buzz.mockClear();
	withVibrate(true);
	rated.length = 0;
	rateOk = true;
	fake.app.playerId = '02:00:00:00:00:01';
	fake.app.players = [{ playerid: '02:00:00:00:00:01', name: 'Test player' }];
	fake.app.now = { mode: 'play', time: 10, duration: 200, playlist_loop: [{ id: 1, title: 'A song', duration: 200 }] };
	fake.app.sleepCalls.length = 0;
});
afterAll(() => {
	withVibrate(false);
	vi.unstubAllGlobals();
});

test('off by default: nothing vibrates until switched on, and the choice is kept', () => {
	expect(haptics.enabled).toBe(false);
	expect(haptics.tick('skip')).toBe(false);
	expect(buzz).not.toHaveBeenCalled();
	haptics.set(true);
	expect(localStorage.getItem(HAPTICS_KEY)).toBe('1');
	haptics.reload();
	expect(haptics.enabled).toBe(true);
	expect(haptics.tick('skip')).toBe(true);
	expect(buzz).toHaveBeenCalledWith(PATTERNS.skip);
	haptics.set(false);
	expect(haptics.tick('skip')).toBe(false);
	expect(buzz).toHaveBeenCalledTimes(1);
});

test('every event is a light tick (short patterns)', () => {
	for (const p of Object.values(PATTERNS)) {
		const total = Array.isArray(p) ? p.filter((_, i) => i % 2 === 0).reduce((a, b) => a + b, 0) : p;
		expect(total).toBeGreaterThan(0);
		expect(total).toBeLessThanOrEqual(25);
	}
});

test('without navigator.vibrate: unsupported, and ticks do nothing', () => {
	withVibrate(false);
	haptics.set(true);
	expect(haptics.supported).toBe(false);
	expect(() => haptics.tick('rating')).not.toThrow();
	expect(haptics.tick('rating')).toBe(false);
});

test('a vibrate that throws (or blocked storage) is harmless', () => {
	Object.defineProperty(navigator, 'vibrate', { value: () => { throw new Error('blocked'); }, configurable: true, writable: true });
	haptics.set(true);
	expect(haptics.tick('drop')).toBe(false);
	const get = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('denied'); });
	haptics.reload();
	expect(haptics.enabled).toBe(false);
	get.mockRestore();
});

test('steps: volume every 5, scrubbing every tenth of the track', () => {
	expect(crossed(10, 14, 5)).toBe(false);
	expect(crossed(14, 15, 5)).toBe(true);
	expect(crossed(15, 14, 5)).toBe(true);
	expect(crossed(1, 2, 0)).toBe(false);
	expect(crossed(NaN, 2, 5)).toBe(false);
	haptics.set(true);
	haptics.volume(50, 52);
	expect(buzz).not.toHaveBeenCalled();
	haptics.volume(52, 55);
	expect(buzz).toHaveBeenCalledWith(PATTERNS.volume);
	buzz.mockClear();
	haptics.scrub(10, 15, 200); // 5 % → 7.5 %: same tenth
	expect(buzz).not.toHaveBeenCalled();
	haptics.scrub(15, 25, 200); // crosses 10 %
	expect(buzz).toHaveBeenCalledWith(PATTERNS.scrub);
	buzz.mockClear();
	haptics.scrub(15, 25, 0); // unknown length: no detents
	expect(buzz).not.toHaveBeenCalled();
});

test('scrubbing the seek bar ticks at each detent', async () => {
	haptics.set(true);
	const oninput = vi.fn();
	const r = render(SeekBar, { value: 10, total: 200, oninput });
	const slider = r.container.querySelector('input[type="range"]')!;
	await fireEvent.input(slider, { target: { value: '15' } });
	expect(buzz).not.toHaveBeenCalled();
	await fireEvent.input(slider, { target: { value: '45' } });
	expect(buzz).toHaveBeenCalledTimes(1);
	await fireEvent.input(slider, { target: { value: '65' } });
	expect(buzz).toHaveBeenCalledTimes(2);
	expect(oninput).toHaveBeenLastCalledWith(65);
});

test('setting a rating ticks; a failed one doesn’t', async () => {
	haptics.set(true);
	render(RatingControl, { track: 5 });
	await fireEvent.click(document.querySelector('[data-star="4"]')!);
	await waitFor(() => expect(buzz).toHaveBeenCalledWith(PATTERNS.rating));
	buzz.mockClear();
	rateOk = false;
	await fireEvent.click(document.querySelector('[data-star="2"]')!);
	await waitFor(() => expect(rated).toHaveLength(2));
	expect(buzz).not.toHaveBeenCalled();
});

test('setting a sleep timer ticks', async () => {
	haptics.set(true);
	render(SleepSheet, { onclose: () => {} });
	await fireEvent.click(screen.getByRole('button', { name: '15 min' }));
	await waitFor(() => expect(fake.app.sleepCalls).toHaveLength(1));
	expect(buzz).toHaveBeenCalledWith(PATTERNS.sleep);
});

test('the Settings switch (Settings › Now playing) turns haptics on and off, and is disabled without the API', async () => {
	vi.stubGlobal('fetch', async () => new Response('{}', { status: 200, headers: { 'content-type': 'application/json' } }));
	const { default: Settings } = await import('#lib/settings/NowPlayingSettings.svelte');
	const r = render(Settings);
	const sw = screen.getByRole('switch', { name: 'Haptics' });
	expect(sw.getAttribute('aria-checked')).toBe('false');
	// The state in words, not colour alone.
	expect(sw.textContent).toContain('Off');
	await fireEvent.click(sw);
	expect(haptics.enabled).toBe(true);
	expect(localStorage.getItem(HAPTICS_KEY)).toBe('1');
	await fireEvent.click(sw);
	expect(haptics.enabled).toBe(false);
	r.unmount();
	withVibrate(false);
	render(Settings);
	const off = screen.getByRole('switch', { name: 'Haptics' }) as HTMLButtonElement;
	expect(off.disabled).toBe(true);
	expect(screen.getByText(/can’t vibrate/)).toBeTruthy();
});
