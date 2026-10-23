// Wave 2 part 3 on Now playing: the Art, Lyrics and Info layouts (the segmented control in wide
// windows, the pager on phones, keys 1–3, sideways swipes), wide windows (pane beside the art,
// queue pinned on the right), and the visualiser's controls (only when this browser is the
// player, off under reduced motion, decorative, watching the player only while shown). The
// phone layout's own controls are in now-playing-phone.test.ts.
import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { flushSync, tick } from 'svelte';
import type { PlayerStatus } from '#lib/lms/types';

vi.mock('#lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const fake = await import('./helpers/fake-app.svelte');
const { choose, menuItems } = await import('./helpers/menu');
const { goto } = await import('$app/navigation');
const { device } = await import('#lib/player/device.svelte');
const { nowPlaying } = await import('#lib/nowplaying/store.svelte');
const { NOW_PLAYING_KEY } = await import('#lib/nowplaying/prefs');
const { default: NowPlaying } = await import('../src/routes/+page.svelte');

const HERE = '02:00:00:00:00:0a';
const KITCHEN = '02:00:00:00:00:0b';

class NoObserver {
	observe() {}
	unobserve() {}
	disconnect() {}
}
let reduceMotion = false;
let width = 600;
beforeAll(() => {
	vi.stubGlobal('IntersectionObserver', NoObserver);
	vi.stubGlobal('ResizeObserver', NoObserver);
	vi.stubGlobal('matchMedia', (q: string) => {
		const min = q.match(/min-width:\s*(\d+)px/);
		return {
			matches: q.includes('reduced-motion') ? reduceMotion : min ? width >= Number(min[1]) : false,
			addEventListener() {},
			removeEventListener() {},
		};
	});
	// jsdom has no canvas: drawing is tested on its own (visual.test.ts).
	HTMLCanvasElement.prototype.getContext = (() => null) as typeof HTMLCanvasElement.prototype.getContext;
});
afterAll(() => vi.unstubAllGlobals());

function status(over: Partial<PlayerStatus> = {}): PlayerStatus {
	return {
		mode: 'play',
		time: 30,
		duration: 240,
		playlist_cur_index: 0,
		playlist_tracks: 2,
		'mixer volume': 50,
		playlist_loop: [
			{ id: 11, title: 'Current song', artist: 'Band', album: 'Record', album_id: 5, type: 'flc', samplesize: '16', samplerate: '44100' },
			{ id: 12, title: 'Next song', artist: 'Other band', duration: 200 },
		],
		...over,
	};
}

function setVisibility(v: 'visible' | 'hidden') {
	Object.defineProperty(document, 'visibilityState', { value: v, configurable: true });
	document.dispatchEvent(new Event('visibilitychange'));
}

beforeEach(() => {
	localStorage.removeItem(NOW_PLAYING_KEY);
	nowPlaying.reload();
	fake.calls.length = 0;
	fake.app.players = [
		{ playerid: HERE, name: 'This browser' },
		{ playerid: KITCHEN, name: 'Kitchen' },
	];
	fake.app.deviceMac = HERE;
	fake.app.playerId = HERE;
	fake.app.now = status();
	device.mac = HERE;
	device.state = 'connected';
	reduceMotion = false;
	width = 600;
	setVisibility('visible');
	fake.handler.rpc = async () => ({});
});

const layoutButtons = (c: Element) => [...c.querySelectorAll<HTMLButtonElement>('[data-layout-button]')];
/** The layout shown: the segmented control's choice (wide), or the pager's page (phones). */
const pressed = (c: Element) =>
	layoutButtons(c).find((b) => b.getAttribute('aria-pressed') === 'true')?.dataset.layoutButton ?? c.querySelector<HTMLElement>('[data-pager]')?.dataset.page;

function pointer(el: Element, type: string, init: PointerEventInit) {
	el.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, pointerType: 'touch', ...init }));
	flushSync();
}
function swipe(el: Element, dx: number, dy = 0) {
	pointer(el, 'pointerdown', { clientX: 200, clientY: 200 });
	pointer(el, 'pointerup', { clientX: 200 + dx, clientY: 200 + dy });
}

describe('layouts', () => {
	test('wide windows: a labelled segmented control picks Art, Lyrics or Info, remembered on this device', async () => {
		width = 1300;
		const r = render(NowPlaying);
		await tick();
		const group = r.container.querySelector('[data-layouts]')!;
		expect(group.getAttribute('role')).toBe('group');
		expect(group.getAttribute('aria-label')).toBe('Now playing layout');
		expect(layoutButtons(r.container).map((b) => b.textContent?.trim())).toEqual(['Art', 'Lyrics', 'Info']);
		expect(layoutButtons(r.container).map((b) => b.getAttribute('aria-keyshortcuts'))).toEqual(['1', '2', '3']);
		expect(pressed(r.container)).toBe('art');
		await fireEvent.click(layoutButtons(r.container)[2]!);
		expect(pressed(r.container)).toBe('info');
		expect(JSON.parse(localStorage.getItem(NOW_PLAYING_KEY)!).layout).toBe('info');
		r.unmount();
	});

	test('keys 1, 2 and 3 choose the layout, but not while typing or with a modifier', async () => {
		const r = render(NowPlaying);
		await tick();
		await fireEvent.keyDown(document.body, { key: '2' });
		expect(pressed(r.container)).toBe('lyrics');
		expect(r.container.querySelector('[data-page-panel="lyrics"] [data-lyrics-pane]')).not.toBeNull();
		await fireEvent.keyDown(document.body, { key: '3' });
		expect(pressed(r.container)).toBe('info');
		await fireEvent.keyDown(document.body, { key: '1', ctrlKey: true });
		expect(pressed(r.container)).toBe('info');
		const seek = r.container.querySelector<HTMLInputElement>('input[type=range]')!;
		await fireEvent.keyDown(seek, { key: '1' });
		expect(pressed(r.container)).toBe('info');
		await fireEvent.keyDown(document.body, { key: '1' });
		expect(pressed(r.container)).toBe('art');
		r.unmount();
	});

	test('a sideways swipe on the pager moves between layouts; a swipe up still peeks at the queue', async () => {
		const r = render(NowPlaying);
		await tick();
		const area = r.container.querySelector('[data-pager]')!;
		swipe(area, -100);
		expect(pressed(r.container)).toBe('lyrics');
		// A swipe over the lyrics goes on to Info.
		swipe(r.container.querySelector('[data-page-panel="lyrics"]')!, -100);
		expect(pressed(r.container)).toBe('info');
		swipe(area, -100);
		expect(pressed(r.container)).toBe('info'); // the last page
		swipe(area, 100);
		swipe(area, 100);
		expect(pressed(r.container)).toBe('art');
		// Too short, or mostly vertical: not a layout swipe.
		swipe(area, -30);
		expect(pressed(r.container)).toBe('art');
		swipe(area, 10, -100);
		expect(pressed(r.container)).toBe('art');
		await waitFor(() => expect(document.querySelector('[data-full-queue]')).not.toBeNull());
		r.unmount();
	});

	test('the signal path is never on screen until asked for', async () => {
		width = 1300;
		const r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('[data-signal-path]')).toBeNull();
		await fireEvent.keyDown(document.body, { key: '3' });
		expect(r.container.querySelector('[data-side-pane="info"]')).not.toBeNull();
		expect(r.container.querySelector('[data-signal-path]')).toBeNull();
		expect(r.container.querySelector('[data-signal-path-toggle]')).not.toBeNull();
		r.unmount();
	});

	test('Info (wide): quality, the signal path on demand, rating, credits and Up next', async () => {
		width = 1300;
		fake.handler.rpc = async (_p, cmd) => (cmd[0] === 'titles' ? { titles_loop: [{ id: 11, composer: 'A Composer', composer_ids: '7' }] } : {});
		const r = render(NowPlaying);
		await tick();
		await fireEvent.click(layoutButtons(r.container)[2]!);
		const pane = r.container.querySelector('[data-info-pane]')!;
		expect(pane.textContent).toContain('FLAC 16/44.1');
		// The signal path is on demand: folded away behind its disclosure until asked for.
		expect(pane.querySelector('[data-signal-path]')).toBeNull();
		const disclose = pane.querySelector<HTMLButtonElement>('[data-signal-path-toggle]')!;
		expect(disclose.textContent).toContain('Signal path');
		expect(disclose.getAttribute('aria-expanded')).toBe('false');
		await fireEvent.click(disclose);
		expect(disclose.getAttribute('aria-expanded')).toBe('true');
		expect(pane.querySelector('[data-signal-path]')?.textContent).toContain('Track in the library');
		expect(pane.querySelector('[data-signal-path]')?.textContent).toContain('Stream to this device');
		await fireEvent.click(disclose);
		expect(pane.querySelector('[data-signal-path]')).toBeNull();
		expect(pane.querySelector('[data-up-next]')?.textContent).toContain('Next song');
		// Up next and the rating move into the pane rather than appearing twice.
		expect(r.container.querySelectorAll('[data-up-next]')).toHaveLength(1);
		expect(fake.calls.some((c) => c.cmd[0] === 'titles' && c.cmd.includes('track_id:11'))).toBe(true);
		r.unmount();
	});

	test('Track info (phones): quality and credits; no signal path inline, and nothing twice', async () => {
		fake.handler.rpc = async (_p, cmd) => (cmd[0] === 'titles' ? { titles_loop: [{ id: 11, composer: 'A Composer', composer_ids: '7' }] } : {});
		const r = render(NowPlaying);
		await tick();
		// Not asked for until the page is shown.
		expect(fake.calls.some((c) => c.cmd[0] === 'titles')).toBe(false);
		await fireEvent.keyDown(document.body, { key: '3' });
		const pane = r.container.querySelector('[data-page-panel="info"] [data-info-pane]')!;
		expect(pane.textContent).toContain('FLAC 16/44.1');
		expect(pane.querySelector('[data-signal-path-toggle]')).toBeNull();
		expect(r.container.querySelector('[data-signal-path]')).toBeNull();
		// The rating and Up next stay where they always are, once each.
		expect(pane.querySelector('[data-rating-stars], [data-rating-heart]')).toBeNull();
		expect(r.container.querySelectorAll('[data-up-next]')).toHaveLength(1);
		await waitFor(() => expect(fake.calls.some((c) => c.cmd[0] === 'titles' && c.cmd.includes('track_id:11'))).toBe(true));
		r.unmount();
	});

	test('wide windows: the art stays, the pane sits beside it, and the queue can be pinned or hidden', async () => {
		width = 1300;
		fake.handler.rpc = async (_p, cmd) => (cmd[0] === 'status' && cmd[1] === 0 ? { playlist_loop: status().playlist_loop } : {});
		const r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('[data-side-pane]')).toBeNull();
		await fireEvent.keyDown(document.body, { key: '2' });
		expect(r.container.querySelector('[data-side-pane="lyrics"]')).not.toBeNull();
		// The cover is still shown on the left.
		expect(r.container.querySelector('.art-box')).not.toBeNull();
		const q = r.container.querySelector('[data-queue-side]')!;
		expect(q.getAttribute('aria-label')).toBe('Queue');
		await waitFor(() => expect(q.querySelector('[data-side-queue="now"]')).not.toBeNull());
		const toggle = r.container.querySelector<HTMLButtonElement>('[data-queue-panel-toggle]')!;
		expect(toggle.getAttribute('aria-pressed')).toBe('true');
		// It is the Queue action itself, not a button of its own under the layout tabs.
		expect(toggle.closest('.modes')).toBeNull();
		expect(r.container.querySelector('.modes [aria-label*="queue" i]')).toBeNull();
		expect(toggle.textContent).toContain('Queue');
		expect(r.container.querySelector('[data-queue-peek]')).toBeNull();
		await fireEvent.click(toggle);
		expect(r.container.querySelector('[data-queue-side]')).toBeNull();
		expect(JSON.parse(localStorage.getItem(NOW_PLAYING_KEY)!).queuePanel).toBe(false);
		r.unmount();
	});

	test('a medium window has no queue panel or its button', async () => {
		width = 900;
		const r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('[data-queue-side]')).toBeNull();
		expect(r.container.querySelector('[data-queue-panel-toggle]')).toBeNull();
		// The Queue action peeks at the queue instead.
		expect(r.container.querySelector('[data-queue-peek]')).not.toBeNull();
		r.unmount();
	});
});

describe('visualiser', () => {
	test('phones: off by default; ⋮ › Visualiser switches it on in a style, and changes it, remembered', async () => {
		const r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('[data-visualiser]')).toBeNull();
		// No button of its own in the bottom row (Wave 3.5): it lives in ⋮.
		expect(r.container.querySelector('[data-visualiser-button]')).toBeNull();
		expect(r.container.querySelector('[data-visualiser-style]')).toBeNull();
		expect(r.container.querySelector('[data-visualiser-toggle]')).toBeNull();
		await fireEvent.click(r.getByRole('button', { name: 'More options' }));
		const row = r.container.querySelector<HTMLButtonElement>('[data-visualiser-style-row]')!;
		expect(row.textContent).toContain('Visualiser');
		expect(row.textContent).toContain('Off');
		await fireEvent.click(row);
		await fireEvent.click(document.querySelector('[data-style-choice="bars"]')!);
		const vis = r.container.querySelector('[data-visualiser]')!;
		expect(vis.getAttribute('aria-hidden')).toBe('true');
		expect(vis.getAttribute('data-visualiser')).toBe('bars');
		expect(device.visualWatchers).toBe(1);
		await fireEvent.click(r.getByRole('button', { name: 'More options' }));
		expect(r.container.querySelector('[data-visualiser-style-row]')!.textContent).toContain('Bars');
		await fireEvent.click(r.container.querySelector('[data-visualiser-style-row]')!);
		await fireEvent.click(document.querySelector('[data-style-choice="scope"]')!);
		expect(r.container.querySelector('[data-visualiser]')!.getAttribute('data-visualiser')).toBe('scope');
		expect(JSON.parse(localStorage.getItem(NOW_PLAYING_KEY)!)).toMatchObject({ visualiser: true, style: 'scope' });
		r.unmount();
		expect(device.visualWatchers).toBe(0);
	});

	test('wide windows: off by default; the toggle and style button are labelled and remembered', async () => {
		width = 1300;
		const r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('[data-visualiser]')).toBeNull();
		const toggle = r.container.querySelector<HTMLButtonElement>('[data-visualiser-toggle]')!;
		expect(toggle.getAttribute('aria-label')).toBe('Visualiser');
		expect(toggle.getAttribute('aria-pressed')).toBe('false');
		await fireEvent.click(toggle);
		expect(toggle.getAttribute('aria-pressed')).toBe('true');
		const vis = r.container.querySelector('[data-visualiser]')!;
		expect(vis.getAttribute('aria-hidden')).toBe('true');
		expect(vis.getAttribute('data-visualiser')).toBe('bars');
		// It asks this device's player for frames only while shown.
		expect(device.visualWatchers).toBe(1);
		const style = r.container.querySelector<HTMLButtonElement>('[data-visualiser-style]')!;
		expect(style.getAttribute('aria-label')).toBe('Visualiser style: Bars. Change style');
		await fireEvent.click(style);
		expect(r.container.querySelector('[data-visualiser]')!.getAttribute('data-visualiser')).toBe('scope');
		expect(JSON.parse(localStorage.getItem(NOW_PLAYING_KEY)!)).toMatchObject({ visualiser: true, style: 'scope' });
		r.unmount();
		expect(device.visualWatchers).toBe(0);
	});

	test('a swipe across it changes the style, not the layout', async () => {
		nowPlaying.setVisualiser(true);
		const r = render(NowPlaying);
		await tick();
		swipe(r.container.querySelector('[data-visualiser]')!, -100);
		expect(r.container.querySelector('[data-visualiser]')!.getAttribute('data-visualiser')).toBe('scope');
		expect(pressed(r.container)).toBe('art');
		swipe(r.container.querySelector('[data-visualiser]')!, 100);
		expect(r.container.querySelector('[data-visualiser]')!.getAttribute('data-visualiser')).toBe('bars');
		r.unmount();
	});

	test('Glow keeps the cover and pulses the background; frames reach it', async () => {
		nowPlaying.setVisualiser(true);
		nowPlaying.setStyle('glow');
		const r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('[data-visualiser="glow"]')).not.toBeNull();
		expect(r.container.querySelector('.art-box img, .art-box [data-art], .art-box')).not.toBeNull();
		expect(device.visualWatchers).toBe(1);
		r.unmount();
	});

	test('Glow: a halo behind the cover, turning only while playing; frames with a beat kick it', async () => {
		nowPlaying.setVisualiser(true);
		nowPlaying.setStyle('glow');
		let feed: ((f: Float32Array) => void) | null = null;
		const watch = vi.spyOn(device, 'watchVisual').mockImplementation((cb) => {
			feed = cb;
			return () => (feed = null);
		});
		const r = render(NowPlaying);
		await tick();
		const halo = r.container.querySelector('.art-box [data-glow-halo]')!;
		expect(halo).not.toBeNull();
		expect(halo.getAttribute('aria-hidden')).toBe('true');
		// Behind the cover, which keeps its own layer above it.
		expect(halo.nextElementSibling?.classList.contains('cover')).toBe(true);
		expect(halo.classList.contains('spinning')).toBe(true);
		// No art to read in jsdom: the theme's accent colours it.
		expect(halo.getAttribute('data-colours')).toBe('0');
		expect(halo.querySelectorAll('.lobe')).toHaveLength(3);
		// The old level-following glow is gone: nothing scales the background.
		const section = r.container.querySelector<HTMLElement>('section.np')!;
		expect(section.style.getPropertyValue('--np-bass')).toBe('');
		expect(feed).not.toBeNull();
		// A quiet bed, then a kick in the low bands: the halo grows and brightens.
		const frame = (bass: number) => {
			const f = new Float32Array(32 + 128 + 5);
			for (let i = 0; i < 32; i++) f[i] = i < 5 ? bass : 0.1;
			f.fill(-30, 160, 164);
			f[164] = bass;
			return f;
		};
		for (let i = 0; i < 6; i++) {
			feed!(frame(0.1));
			await new Promise((res) => setTimeout(res, 20));
		}
		feed!(frame(0.9));
		await waitFor(() => expect(Number(section.style.getPropertyValue('--glow-radius'))).toBeGreaterThan(1.1));
		expect(Number(section.style.getPropertyValue('--glow-opacity'))).toBeGreaterThan(0.5);
		expect(Number(section.style.getPropertyValue('--glow-cover'))).toBeGreaterThan(1);
		expect(Number(section.style.getPropertyValue('--glow-cover'))).toBeLessThanOrEqual(1.02);
		// Paused: the halo stops turning.
		fake.app.now = status({ mode: 'pause' });
		flushSync();
		expect(r.container.querySelector('[data-glow-halo]')!.classList.contains('spinning')).toBe(false);
		r.unmount();
		watch.mockRestore();
	});

	test('Glow under reduced motion: a still halo, no pulse and no turning', async () => {
		reduceMotion = true;
		nowPlaying.setVisualiser(true);
		nowPlaying.setStyle('glow');
		const r = render(NowPlaying);
		await tick();
		const halo = r.container.querySelector('[data-glow-halo]')!;
		expect(halo.classList.contains('still')).toBe(true);
		expect(halo.classList.contains('spinning')).toBe(false);
		expect(r.container.querySelector('[data-visualiser]')).toBeNull();
		expect(device.visualWatchers).toBe(0);
		r.unmount();
	});

	test('Glow: none for a remote player, nor for the other styles', async () => {
		nowPlaying.setVisualiser(true);
		nowPlaying.setStyle('glow');
		fake.app.playerId = KITCHEN;
		let r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('[data-glow-halo]')).toBeNull();
		expect(r.container.querySelector('[data-ambient]')).not.toBeNull();
		r.unmount();
		fake.app.playerId = HERE;
		nowPlaying.setStyle('bars');
		r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('[data-glow-halo]')).toBeNull();
		r.unmount();
	});

	test('it stops watching while the page is hidden, and starts again when shown', async () => {
		nowPlaying.setVisualiser(true);
		const r = render(NowPlaying);
		await tick();
		expect(device.visualWatchers).toBe(1);
		setVisibility('hidden');
		expect(device.visualWatchers).toBe(0);
		setVisibility('visible');
		expect(device.visualWatchers).toBe(1);
		r.unmount();
	});

	test('reduced motion: off, and its toggle says why', async () => {
		reduceMotion = true;
		nowPlaying.setVisualiser(true);
		const r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('[data-visualiser]')).toBeNull();
		expect(device.visualWatchers).toBe(0);
		await fireEvent.click(r.getByRole('button', { name: 'More options' }));
		const toggle = r.container.querySelector<HTMLButtonElement>('[data-visualiser-style-row]')!;
		expect(toggle.disabled).toBe(true);
		expect(toggle.textContent).toMatch(/reduced motion/);
		expect(r.container.querySelector('[data-visualiser-style]')).toBeNull();
		r.unmount();
	});

	test('a remote player: no visualiser and no controls for it; the ambient background stays', async () => {
		nowPlaying.setVisualiser(true);
		fake.app.playerId = KITCHEN;
		const r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('[data-visualiser]')).toBeNull();
		expect(r.container.querySelector('[data-visualiser-button]')).toBeNull();
		expect(r.container.querySelector('[data-visualiser-toggle]')).toBeNull();
		expect(r.container.querySelector('[data-ambient]')).not.toBeNull();
		expect(device.visualWatchers).toBe(0);
		r.unmount();
	});

	test('not shown in the Lyrics or Info layouts on a narrow screen (the art is not there)', async () => {
		nowPlaying.setVisualiser(true);
		const r = render(NowPlaying);
		await tick();
		await fireEvent.keyDown(document.body, { key: '2' });
		expect(r.container.querySelector('[data-visualiser]')).toBeNull();
		expect(device.visualWatchers).toBe(0);
		r.unmount();
	});
});

test('car mode opens from ⋯ on wide Now playing (Wave 2 §5.1; in ⋯ since Wave 3.5)', async () => {
	width = 1300;
	const r = render(NowPlaying);
	expect(r.queryByRole('link', { name: 'Car mode' })).toBeNull();
	await choose('More options', 'Car mode', r.container);
	expect(goto).toHaveBeenCalledWith('#/car', expect.anything());
	r.unmount();
	fake.app.playerId = '';
	const none = render(NowPlaying);
	expect(await menuItems('More options', none.container)).not.toContain('Car mode');
	none.unmount();
});

test('car mode, on phones: in the ⋮ menu', async () => {
	const r = render(NowPlaying);
	expect(r.queryByRole('link', { name: /Car mode/ })).toBeNull();
	await fireEvent.click(r.getByRole('button', { name: 'More options' }));
	expect(r.getByRole('link', { name: /Car mode/ }).getAttribute('href')).toBe('#/car');
	r.unmount();
});
