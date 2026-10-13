// Now playing on phones (up to 840 px): the slim top row (close, "Playing on …", ⋮ and its
// sheet), the pager of Cover, Lyrics and Track info (swipes, keys, dots), the one Visualiser
// button (tap, long press, right-click, Shift+F10, swipe across it, the style's name read out),
// the Volume popover (another player or this browser), and the layout's order. The CSS that
// keeps it from scrolling is checked in tests/node/now-playing-layout.test.mjs.
import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { flushSync, tick } from 'svelte';
import type { PlayerStatus } from '$lib/lms/types';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const fake = await import('./helpers/fake-app.svelte');
const { device } = await import('$lib/player/device.svelte');
const { nowPlaying } = await import('$lib/nowplaying/store.svelte');
const { NOW_PLAYING_KEY } = await import('$lib/nowplaying/prefs');
const { PAGE_LABELS, pageForKey, pagerGesture, stepPage } = await import('$lib/nowplaying/pager');
const { goto } = await import('$app/navigation');
const { default: NowPlaying } = await import('../src/routes/+page.svelte');
const { default: VisualiserButton } = await import('$lib/components/VisualiserButton.svelte');

const HERE = '02:00:00:00:00:0a';
const KITCHEN = '02:00:00:00:00:0b';

class NoObserver {
	observe() {}
	unobserve() {}
	disconnect() {}
}
let reduceMotion = false;
let width = 360;
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
		'mixer volume': 40,
		playlist_loop: [
			{ id: 11, title: 'Harbour Lights', artist: 'The Example Quartet', artist_id: 3, album: 'Coastal Hours', album_id: 5, type: 'flc', samplesize: '24', samplerate: '96000' },
			{ id: 12, title: 'Lantern Street', artist: 'The Example Quartet', duration: 200 },
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
	device.volumePlan = null;
	reduceMotion = false;
	width = 360;
	setVisibility('visible');
	fake.handler.rpc = async () => ({});
});

function pointer(el: Element, type: string, init: PointerEventInit) {
	el.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, pointerType: 'touch', ...init }));
	flushSync();
}
function swipe(el: Element, dx: number, dy = 0, pointerType = 'touch') {
	pointer(el, 'pointerdown', { clientX: 200, clientY: 200, pointerType });
	pointer(el, 'pointerup', { clientX: 200 + dx, clientY: 200 + dy, pointerType });
}
const page = (c: Element) => c.querySelector<HTMLElement>('[data-pager]')!.dataset.page;
const sent = () => fake.calls.filter((c) => c.cmd[0] !== 'status' && c.cmd[0] !== 'players' && c.cmd[0] !== 'titles');

describe('the pager: helpers', () => {
	test('a sideways swipe pages, a swipe up over the cover peeks; short, diagonal or junk does nothing', () => {
		expect(pagerGesture(-80, 5, 'touch')).toBe('next');
		expect(pagerGesture(80, -5, 'mouse')).toBe('previous');
		expect(pagerGesture(-30, 0, 'touch')).toBeNull();
		expect(pagerGesture(-60, 55, 'touch')).toBeNull();
		expect(pagerGesture(4, -90, 'touch')).toBe('peek');
		expect(pagerGesture(4, -90, 'mouse')).toBeNull();
		expect(pagerGesture(4, 90, 'touch')).toBeNull();
		for (const bad of [NaN, Infinity, -Infinity]) expect(pagerGesture(bad, 0, 'touch')).toBeNull();
	});

	test('keys and steps stay within the three pages', () => {
		expect(pageForKey('art', 'ArrowRight')).toBe('lyrics');
		expect(pageForKey('info', 'ArrowRight')).toBe('info');
		expect(pageForKey('art', 'ArrowLeft')).toBe('art');
		expect(pageForKey('lyrics', 'End')).toBe('info');
		expect(pageForKey('info', 'Home')).toBe('art');
		expect(pageForKey('art', 'Enter')).toBeNull();
		expect(stepPage('lyrics', 1)).toBe('info');
		expect(stepPage('art', -1)).toBe('art');
		expect(stepPage('bogus' as never, 1)).toBe('lyrics');
		expect(Object.values(PAGE_LABELS)).toEqual(['Cover', 'Lyrics', 'Track info']);
	});
});

describe('layout', () => {
	test('top to bottom: top row, pager, title, seek, transport, Up next, four actions; nothing of the old phone page', async () => {
		const r = render(NowPlaying);
		await tick();
		const root = r.container.querySelector('[data-np-phone]')!;
		expect(root).not.toBeNull();
		const order = ['[data-np-close]', '[data-play-on]', '[data-np-more]', '[data-pager]', '[data-page-dots]', 'h1.title', '[data-quality-badge]', 'input[aria-label="Position"]', '[aria-label="Previous"]', '[data-up-next]', '[data-sleep-button]', '[data-queue-peek]', '[data-visualiser-button]', '[data-volume-button]'];
		const els = order.map((q) => root.querySelector(q));
		els.forEach((e, i) => expect(e, order[i]).not.toBeNull());
		for (let i = 1; i < els.length; i++) {
			expect(els[i - 1]!.compareDocumentPosition(els[i]!) & Node.DOCUMENT_POSITION_FOLLOWING, `${order[i - 1]} before ${order[i]}`).toBeTruthy();
		}
		// Only the hero is inside the pager: the controls are always on screen.
		const pager = root.querySelector('[data-pager]')!;
		for (const q of ['h1.title', '[aria-label="Previous"]', '[data-up-next]', '[data-volume-button]']) expect(pager.contains(root.querySelector(q))).toBe(false);
		// The transport: shuffle, previous, play/pause, next, repeat.
		const transport = root.querySelector('.transport')!;
		expect([...transport.querySelectorAll('button')].map((b) => b.getAttribute('aria-label'))).toEqual(['Shuffle: off', 'Previous', 'Pause', 'Next', 'Repeat: off']);
		// The bottom row: four actions in a labelled group.
		const actions = r.getByRole('navigation', { name: 'Now playing actions' });
		expect([...actions.querySelectorAll(':scope > button, :scope > span > button')].map((b) => b.getAttribute('aria-label'))).toEqual(['Sleep timer', 'Queue', 'Visualiser', 'Volume on this device']);
		// Gone: the segmented control, the full-width volume row, the separate style button, and
		// the car, alarms and lyrics buttons (now in ⋮ or the pager).
		for (const q of ['[data-layouts]', '.volrow', '[data-visualiser-style]', '[data-visualiser-toggle]', '[data-car-link]', '[data-alarms-link]', '[data-lyrics-button]', '[data-lyrics-peek]', 'input[type=range][max="100"]']) {
			expect(root.querySelector(q), q).toBeNull();
		}
		r.unmount();
	});

	test('the title is one line with the rating beside it; artist · album and the quality badge under it', async () => {
		const r = render(NowPlaying);
		await tick();
		const row = r.container.querySelector('.title-row')!;
		expect(row.querySelector('h1')!.textContent).toBe('Harbour Lights');
		expect(row.querySelector('h1')!.getAttribute('title')).toBe('Harbour Lights');
		expect(row.querySelector('[data-rating-stars], [data-rating-heart]')).not.toBeNull();
		const sub = r.container.querySelector('.sub')!;
		expect(sub.querySelector('.who')!.textContent!.replace(/\s+/g, ' ').trim()).toBe('The Example Quartet · Coastal Hours');
		expect(sub.querySelector('a[href="#/artist/3"]')).not.toBeNull();
		const badge = sub.querySelector<HTMLButtonElement>('[data-quality-badge]')!;
		expect(badge.textContent).toBe('FLAC 24/96 · Hi-Res');
		// The signal path is never shown inline: the badge opens it in a sheet.
		expect(r.container.querySelector('[data-signal-path]')).toBeNull();
		await fireEvent.click(badge);
		await waitFor(() => expect(document.querySelector('dialog[open] [data-signal-path]')).not.toBeNull());
		r.unmount();
	});

	test('a stream has no rating and no lyrics page content; the controls stay', async () => {
		fake.app.now = status({ playlist_loop: [{ title: 'Radio Example', remote: 1 }] });
		const r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('[data-rating-stars], [data-rating-heart]')).toBeNull();
		expect(r.getByRole('button', { name: 'Pause' })).toBeTruthy();
		r.unmount();
	});

	test('nothing to control: says so, and the chip asks for a player', async () => {
		fake.app.playerId = '';
		fake.app.now = null;
		const r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('h1')!.textContent).toBe('Nothing to control yet');
		expect(r.container.querySelector('[data-play-on]')!.textContent).toContain('Choose a player');
		expect((r.getByRole('button', { name: 'Play' }) as HTMLButtonElement).disabled).toBe(true);
		r.unmount();
	});

	test('wide windows (over 840 px) keep their own layout', async () => {
		width = 900;
		const r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('[data-np-phone]')).toBeNull();
		expect(r.container.querySelector('[data-layouts]')).not.toBeNull();
		expect(r.container.querySelector('.volrow')).not.toBeNull();
		r.unmount();
		width = 840;
		const n = render(NowPlaying);
		await tick();
		expect(n.container.querySelector('[data-np-phone]')).not.toBeNull();
		n.unmount();
	});
});

describe('top row', () => {
	test('Close goes back to the previous page (or Home, opened directly)', async () => {
		vi.mocked(goto).mockClear();
		const r = render(NowPlaying);
		await tick();
		const close = r.getByRole('button', { name: 'Close Now playing' });
		await fireEvent.click(close);
		expect(goto).toHaveBeenCalledWith('#/home', expect.objectContaining({ replaceState: true }));
		r.unmount();
	});

	test('"Playing on …" names the player and opens the player picker; Escape closes it', async () => {
		let r = render(NowPlaying);
		await tick();
		const chip = r.container.querySelector<HTMLButtonElement>('[data-play-on]')!;
		expect(chip.textContent).toContain('Playing on this device');
		expect(chip.getAttribute('aria-haspopup')).toBe('dialog');
		expect(chip.getAttribute('aria-expanded')).toBe('false');
		await fireEvent.click(chip);
		expect(chip.getAttribute('aria-expanded')).toBe('true');
		const picker = r.container.querySelector('[role=dialog][aria-label="Play on"]')!;
		expect(picker).not.toBeNull();
		await fireEvent.keyDown(picker, { key: 'Escape' });
		expect(r.container.querySelector('[role=dialog][aria-label="Play on"]')).toBeNull();
		r.unmount();
		fake.app.playerId = KITCHEN;
		r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('[data-play-on]')!.textContent).toContain('Playing on Kitchen');
		r.unmount();
	});

	test('⋮ opens a sheet: car mode, alarms, all rooms, signal path and visualiser style', async () => {
		const r = render(NowPlaying);
		await tick();
		const more = r.getByRole('button', { name: 'More options' });
		more.focus();
		await fireEvent.click(more);
		const sheet = document.querySelector('dialog[open] [data-more-sheet]')!;
		expect(sheet).not.toBeNull();
		expect(sheet.textContent).toMatch(/This track.*Car mode.*Alarms.*All rooms.*Signal path.*Visualiser style/);
		expect(sheet.querySelector('[data-car-link]')!.getAttribute('href')).toBe('#/car');
		expect(sheet.querySelector('[data-alarms-link]')!.getAttribute('href')).toBe('#/alarms');
		expect(sheet.querySelector('[data-rooms-link]')!.getAttribute('href')).toBe('#/rooms');
		// Signal path: the sheet gives way to the signal path's; closing it returns to ⋮.
		await fireEvent.click(sheet.querySelector('[data-signal-path-row]')!);
		await waitFor(() => expect(document.querySelector('dialog[open] [data-signal-path]')).not.toBeNull());
		expect(document.querySelector('[data-more-sheet]')).toBeNull();
		document.querySelector<HTMLDialogElement>('dialog[open]')!.close();
		await waitFor(() => expect(document.querySelector('[data-signal-path]')).toBeNull());
		await waitFor(() => expect(document.activeElement).toBe(r.getByRole('button', { name: 'More options' })));
		r.unmount();
	});

	test('⋮ › Visualiser style opens the chooser; a remote player has no such row', async () => {
		let r = render(NowPlaying);
		await tick();
		await fireEvent.click(r.getByRole('button', { name: 'More options' }));
		await fireEvent.click(document.querySelector('[data-visualiser-style-row]')!);
		expect(document.querySelector('dialog[open] [data-style-sheet]')).not.toBeNull();
		await fireEvent.click(document.querySelector('[data-style-choice="vu"]')!);
		expect(nowPlaying.visualiser).toBe(true);
		expect(nowPlaying.style).toBe('vu');
		r.unmount();
		fake.app.playerId = KITCHEN;
		r = render(NowPlaying);
		await tick();
		await fireEvent.click(r.getByRole('button', { name: 'More options' }));
		expect(document.querySelector('[data-visualiser-style-row]')).toBeNull();
		expect(document.querySelector('[data-rooms-link]')).not.toBeNull();
		r.unmount();
	});
});

describe('pager', () => {
	test('dots are a tab list for Cover, Lyrics and Track info, tied to their panels', async () => {
		const r = render(NowPlaying);
		await tick();
		const list = r.getByRole('tablist', { name: 'Show' });
		const tabs = [...list.querySelectorAll<HTMLButtonElement>('[role=tab]')];
		expect(tabs.map((t) => t.getAttribute('aria-label'))).toEqual(['Cover', 'Lyrics', 'Track info']);
		expect(tabs.map((t) => t.getAttribute('aria-selected'))).toEqual(['true', 'false', 'false']);
		expect(tabs.map((t) => t.tabIndex)).toEqual([0, -1, -1]);
		for (const t of tabs) {
			const panel = document.getElementById(t.getAttribute('aria-controls')!)!;
			expect(panel.getAttribute('role')).toBe('tabpanel');
			expect(panel.getAttribute('aria-labelledby')).toBe(t.id);
		}
		await fireEvent.click(tabs[2]!);
		expect(page(r.container)).toBe('info');
		expect(tabs[2]!.getAttribute('aria-selected')).toBe('true');
		// The pages not shown are inert and hidden from screen readers.
		const art = r.container.querySelector<HTMLElement>('[data-page-panel="art"]')!;
		expect(art.inert || art.hasAttribute('inert')).toBe(true);
		expect(art.getAttribute('aria-hidden')).toBe('true');
		const info = r.container.querySelector<HTMLElement>('[data-page-panel="info"]')!;
		expect(info.hasAttribute('inert')).toBe(false);
		expect(JSON.parse(localStorage.getItem(NOW_PLAYING_KEY)!).layout).toBe('info');
		r.unmount();
	});

	test('arrow keys on the dots move and focus the next tab; on the pager they move it, not the music', async () => {
		const r = render(NowPlaying);
		await tick();
		const cover = r.getByRole('tab', { name: 'Cover' });
		cover.focus();
		await fireEvent.keyDown(cover, { key: 'ArrowRight' });
		expect(page(r.container)).toBe('lyrics');
		expect(document.activeElement).toBe(r.getByRole('tab', { name: 'Lyrics' }));
		await fireEvent.keyDown(document.activeElement!, { key: 'End' });
		expect(page(r.container)).toBe('info');
		await fireEvent.keyDown(document.activeElement!, { key: 'Home' });
		expect(page(r.container)).toBe('art');
		const pager = r.container.querySelector<HTMLElement>('[data-pager]')!;
		expect(pager.tabIndex).toBe(0);
		const e = new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true });
		pager.dispatchEvent(e);
		flushSync();
		expect(page(r.container)).toBe('lyrics');
		// Taken by the pager: the app's ← / → seek shortcut leaves it alone.
		expect(e.defaultPrevented).toBe(true);
		await fireEvent.keyDown(pager, { key: 'ArrowLeft' });
		expect(page(r.container)).toBe('art');
		// The 1, 2 and 3 keys still choose a page.
		await fireEvent.keyDown(document.body, { key: '3' });
		expect(page(r.container)).toBe('info');
		r.unmount();
	});

	test('swipes (touch or mouse) move it; a swipe across a visualiser changes the style instead, but Glow lets it through', async () => {
		const r = render(NowPlaying);
		await tick();
		const pager = r.container.querySelector('[data-pager]')!;
		swipe(pager, -120, 0, 'mouse');
		expect(page(r.container)).toBe('lyrics');
		swipe(pager, 120);
		expect(page(r.container)).toBe('art');
		// A right-button drag is not a swipe.
		pointer(pager, 'pointerdown', { clientX: 200, clientY: 200, pointerType: 'mouse', button: 2 });
		pointer(pager, 'pointerup', { clientX: 60, clientY: 200, pointerType: 'mouse', button: 2 });
		expect(page(r.container)).toBe('art');
		await fireEvent.click(r.container.querySelector('[data-visualiser-button]')!);
		swipe(r.container.querySelector('[data-visualiser="bars"]')!, -120);
		expect(page(r.container)).toBe('art');
		expect(nowPlaying.style).toBe('scope');
		nowPlaying.setStyle('glow');
		flushSync();
		swipe(r.container.querySelector('[data-visualiser="glow"]')!, -120);
		expect(page(r.container)).toBe('lyrics');
		expect(nowPlaying.style).toBe('glow');
		r.unmount();
	});

	test('a page’s content is made when it is first shown', async () => {
		const r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('[data-page-panel="info"] [data-info-pane]')).toBeNull();
		expect(r.container.querySelector('[data-page-panel="lyrics"] [data-lyrics-pane]')).toBeNull();
		await fireEvent.click(r.getByRole('tab', { name: 'Track info' }));
		expect(r.container.querySelector('[data-page-panel="info"] [data-info-pane]')).not.toBeNull();
		await fireEvent.click(r.getByRole('tab', { name: 'Cover' }));
		// Kept once made.
		expect(r.container.querySelector('[data-page-panel="info"] [data-info-pane]')).not.toBeNull();
		r.unmount();
	});

	test('under reduced motion it does not slide', async () => {
		reduceMotion = true;
		const r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('[data-pager] .track')!.classList.contains('still')).toBe(true);
		r.unmount();
	});
});

describe('the Visualiser button', () => {
	test('a tap switches it on (going to the cover) and off; the style’s name is read out', async () => {
		nowPlaying.setLayout('lyrics');
		const r = render(NowPlaying);
		await tick();
		const btn = r.container.querySelector<HTMLButtonElement>('[data-visualiser-button]')!;
		const toast = r.container.querySelector('[data-np-toast]')!;
		expect(toast.getAttribute('role')).toBe('status');
		expect(toast.getAttribute('aria-live')).toBe('polite');
		expect(document.getElementById(btn.getAttribute('aria-describedby')!)!.textContent).toMatch(/Long-press/);
		await fireEvent.click(btn);
		expect(page(r.container)).toBe('art');
		expect(btn.getAttribute('aria-pressed')).toBe('true');
		expect(btn.textContent).toContain('Bars');
		expect(toast.textContent).toBe('Bars');
		expect(r.container.querySelector('[data-visualiser="bars"]')).not.toBeNull();
		await fireEvent.click(btn);
		expect(btn.getAttribute('aria-pressed')).toBe('false');
		expect(toast.textContent).toBe('Visualiser off');
		expect(r.container.querySelector('[data-visualiser]')).toBeNull();
		r.unmount();
	});

	test('the name shows briefly, then goes', async () => {
		vi.useFakeTimers();
		try {
			const r = render(NowPlaying);
			await tick();
			await fireEvent.click(r.container.querySelector('[data-visualiser-button]')!);
			const toast = r.container.querySelector('[data-np-toast]')!;
			expect(toast.classList.contains('shown')).toBe(true);
			vi.advanceTimersByTime(2000);
			flushSync();
			expect(toast.textContent).toBe('');
			expect(toast.classList.contains('shown')).toBe(false);
			r.unmount();
		} finally {
			vi.useRealTimers();
		}
	});

	test('a swipe across it changes the style, and says which', async () => {
		nowPlaying.setVisualiser(true);
		const r = render(NowPlaying);
		await tick();
		swipe(r.container.querySelector('[data-visualiser]')!, -100);
		expect(r.container.querySelector('[data-visualiser]')!.getAttribute('data-visualiser')).toBe('scope');
		expect(r.container.querySelector('[data-np-toast]')!.textContent).toBe('Scope');
		r.unmount();
	});

	test('a long press opens the style chooser and does not also switch it', async () => {
		vi.useFakeTimers();
		try {
			const r = render(NowPlaying);
			await tick();
			const btn = r.container.querySelector<HTMLButtonElement>('[data-visualiser-button]')!;
			pointer(btn, 'pointerdown', { clientX: 10, clientY: 10 });
			vi.advanceTimersByTime(600);
			flushSync();
			const chooser = document.querySelector('dialog[open] [data-style-sheet]')!;
			expect(chooser).not.toBeNull();
			expect(chooser.getAttribute('aria-label')).toBe('Visualiser style');
			pointer(btn, 'pointerup', { clientX: 10, clientY: 10 });
			await fireEvent.click(btn);
			expect(nowPlaying.visualiser).toBe(false);
			// Four styles; the chosen one switches it on.
			expect([...chooser.querySelectorAll('[data-style-choice]')].map((b) => b.textContent!.trim())).toEqual(['Bars', 'Scope', 'Glow', 'VU meters']);
			await fireEvent.click(chooser.querySelector('[data-style-choice="glow"]')!);
			expect(nowPlaying.visualiser).toBe(true);
			expect(nowPlaying.style).toBe('glow');
			expect(document.querySelector('[data-style-sheet]')).toBeNull();
			r.unmount();
		} finally {
			vi.useRealTimers();
		}
	});

	test('a short press is a tap, not a long press', async () => {
		vi.useFakeTimers();
		try {
			const r = render(NowPlaying);
			await tick();
			const btn = r.container.querySelector<HTMLButtonElement>('[data-visualiser-button]')!;
			pointer(btn, 'pointerdown', { clientX: 10, clientY: 10 });
			vi.advanceTimersByTime(200);
			pointer(btn, 'pointerup', { clientX: 10, clientY: 10 });
			await fireEvent.click(btn);
			vi.advanceTimersByTime(600);
			flushSync();
			expect(document.querySelector('[data-style-sheet]')).toBeNull();
			expect(nowPlaying.visualiser).toBe(true);
			r.unmount();
		} finally {
			vi.useRealTimers();
		}
	});

	test('right-click and Shift+F10 open the chooser too; Switch off is offered while on; focus returns', async () => {
		nowPlaying.setVisualiser(true);
		const r = render(NowPlaying);
		await tick();
		const btn = r.container.querySelector<HTMLButtonElement>('[data-visualiser-button]')!;
		const menu = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, button: 2 });
		btn.dispatchEvent(menu);
		flushSync();
		expect(menu.defaultPrevented).toBe(true);
		expect(document.querySelector('dialog[open] [data-style-sheet]')).not.toBeNull();
		expect(document.querySelector('[data-style-choice="bars"]')!.getAttribute('aria-pressed')).toBe('true');
		document.querySelector<HTMLDialogElement>('dialog[open]')!.close();
		await waitFor(() => expect(document.querySelector('[data-style-sheet]')).toBeNull());
		await waitFor(() => expect(document.activeElement).toBe(btn));
		await fireEvent.keyDown(btn, { key: 'F10', shiftKey: true });
		await fireEvent.click(document.querySelector('[data-style-off]')!);
		expect(nowPlaying.visualiser).toBe(false);
		expect(r.container.querySelector('[data-np-toast]')!.textContent).toBe('Visualiser off');
		r.unmount();
	});

	test('hidden for a remote player; disabled, saying why, under reduced motion', async () => {
		fake.app.playerId = KITCHEN;
		let r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('[data-visualiser-button]')).toBeNull();
		r.unmount();
		fake.app.playerId = HERE;
		reduceMotion = true;
		r = render(NowPlaying);
		await tick();
		const btn = r.container.querySelector<HTMLButtonElement>('[data-visualiser-button]')!;
		expect(btn.disabled).toBe(true);
		expect(btn.getAttribute('aria-label')).toMatch(/reduced motion/);
		expect(document.getElementById(btn.getAttribute('aria-describedby')!)!.textContent).toMatch(/reduced motion/);
		r.unmount();
	});

	test('on its own: a context menu while disabled does nothing; a stale long press does not swallow a later tap', async () => {
		const toggles: number[] = [];
		const chooses: number[] = [];
		let r = render(VisualiserButton, { on: false, style: 'Bars', reduce: true, ontoggle: () => toggles.push(1), onchoose: () => chooses.push(1) });
		const off = r.container.querySelector('button')!;
		off.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }));
		await fireEvent.keyDown(off, { key: 'F10', shiftKey: true });
		expect(chooses).toEqual([]);
		r.unmount();
		chooses.length = 0;
		vi.useFakeTimers();
		try {
			r = render(VisualiserButton, { on: false, style: 'Bars', ontoggle: () => toggles.push(1), onchoose: () => chooses.push(1) });
			const btn = r.container.querySelector('button')!;
			pointer(btn, 'pointerdown', { clientX: 0, clientY: 0 });
			vi.advanceTimersByTime(600);
			pointer(btn, 'pointerup', { clientX: 0, clientY: 0 });
			// No click came with that lift (it ended over the chooser); much later, a tap works.
			vi.advanceTimersByTime(5000);
			await fireEvent.click(btn);
			expect(chooses).toEqual([1]);
			expect(toggles).toEqual([1]);
			r.unmount();
		} finally {
			vi.useRealTimers();
		}
	});
});

describe('the Volume popover', () => {
	const open = async (c: HTMLElement) => {
		await fireEvent.click(c.querySelector('[data-volume-button]')!);
		return c.querySelector<HTMLElement>('[data-volume-popover]')!;
	};

	test('another player: labelled with its name, and it sets that player’s volume', async () => {
		fake.app.playerId = KITCHEN;
		const r = render(NowPlaying);
		await tick();
		const btn = r.container.querySelector<HTMLButtonElement>('[data-volume-button]')!;
		expect(btn.getAttribute('aria-label')).toBe('Volume on Kitchen');
		expect(btn.getAttribute('aria-expanded')).toBe('false');
		const pop = await open(r.container);
		expect(btn.getAttribute('aria-expanded')).toBe('true');
		expect(pop.getAttribute('role')).toBe('dialog');
		expect(pop.getAttribute('aria-label')).toBe('Volume on Kitchen');
		const slider = pop.querySelector<HTMLInputElement>('input[type=range]')!;
		expect(slider.getAttribute('aria-label')).toBe('Volume on Kitchen');
		expect(slider.value).toBe('40');
		await waitFor(() => expect(document.activeElement).toBe(slider));
		await fireEvent.input(slider, { target: { value: '65' } });
		await waitFor(() => expect(sent()).toEqual([{ player: KITCHEN, cmd: ['mixer', 'volume', 65] }]));
		expect(pop.textContent).toContain('Kitchen');
		r.unmount();
	});

	test('this browser is the player: it sets this browser’s volume (through LMS, as the slider always has)', async () => {
		const r = render(NowPlaying);
		await tick();
		const pop = await open(r.container);
		expect(pop.getAttribute('aria-label')).toBe('Volume on this device');
		expect(pop.textContent).toContain('This device');
		await fireEvent.input(pop.querySelector('input')!, { target: { value: '20' } });
		await waitFor(() => expect(sent()).toEqual([{ player: HERE, cmd: ['mixer', 'volume', 20] }]));
		r.unmount();
	});

	test('Escape closes it and focus returns to the button; so does a tap outside (without the focus)', async () => {
		const r = render(NowPlaying);
		await tick();
		const btn = r.container.querySelector<HTMLButtonElement>('[data-volume-button]')!;
		let pop = await open(r.container);
		await fireEvent.keyDown(pop.querySelector('input')!, { key: 'Escape' });
		expect(r.container.querySelector('[data-volume-popover]')).toBeNull();
		expect(document.activeElement).toBe(btn);
		pop = await open(r.container);
		pointer(r.container.querySelector('h1')!, 'pointerdown', { clientX: 1, clientY: 1 });
		expect(r.container.querySelector('[data-volume-popover]')).toBeNull();
		// The button toggles it.
		await open(r.container);
		await fireEvent.click(btn);
		expect(r.container.querySelector('[data-volume-popover]')).toBeNull();
		r.unmount();
	});

	test('out-of-range or odd volumes are clamped before they are sent', async () => {
		fake.app.now = status({ 'mixer volume': -30 }); // LMS reports a muted player as negative
		const r = render(NowPlaying);
		await tick();
		const pop = await open(r.container);
		expect(pop.querySelector<HTMLInputElement>('input')!.value).toBe('30');
		await fireEvent.input(pop.querySelector('input')!, { target: { value: '250' } });
		await waitFor(() => expect(sent()).toEqual([{ player: HERE, cmd: ['mixer', 'volume', 100] }]));
		r.unmount();
	});
});

describe('the other actions', () => {
	test('Sleep shows the time left when set; Queue opens the queue peek', async () => {
		fake.app.now = status({ will_sleep_in: 600 });
		const r = render(NowPlaying);
		await tick();
		const sleep = r.container.querySelector('[data-sleep-button]')!;
		expect(sleep.textContent).toContain('10 min');
		expect(sleep.getAttribute('aria-label')).toBe('Sleep timer: stops in 10 min');
		await fireEvent.click(r.container.querySelector('[data-queue-peek]')!);
		await waitFor(() => expect(document.querySelector('dialog[open] [data-full-queue]')).not.toBeNull());
		r.unmount();
	});

	test('Up next is one compact line with its remove button', async () => {
		const r = render(NowPlaying);
		await tick();
		const next = r.container.querySelector('[data-up-next]')!;
		expect(next.textContent).toContain('Up next');
		expect(next.textContent).toContain('Lantern Street');
		expect(next.textContent).toContain('— The Example Quartet');
		expect(r.getByRole('button', { name: 'Remove Lantern Street from the queue' })).toBeTruthy();
		r.unmount();
	});
});
