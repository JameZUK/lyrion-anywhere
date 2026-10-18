// Now playing on phones (up to 840 px), as decluttered in Wave 3.5: the slim top row (close and
// ⋮, whose sheet holds This track, Sleep timer, Visualiser, Car mode and Alarms), the pager of
// Cover, Lyrics and Track info (swipes, keys, dots), one rating toggle by the title, a one-line
// Up next, and the bottom row of three actions: Play on (naming the player, and holding the
// volume), Lyrics and Queue. The CSS that keeps it from scrolling is checked in
// tests/node/now-playing-layout.test.mjs.
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
	test('top to bottom: top row, pager, title, seek, transport, Up next, three actions; nothing of the old phone page', async () => {
		const r = render(NowPlaying);
		await tick();
		const root = r.container.querySelector('[data-np-phone]')!;
		expect(root).not.toBeNull();
		const order = ['[data-np-close]', '[data-np-more]', '[data-pager]', '[data-page-dots]', 'h1.title', '[data-quality-badge]', 'input[aria-label="Position"]', '[aria-label="Previous"]', '[data-up-next]', '[data-play-on]', '[data-lyrics-button]', '[data-queue-peek]'];
		const els = order.map((q) => root.querySelector(q));
		els.forEach((e, i) => expect(e, order[i]).not.toBeNull());
		for (let i = 1; i < els.length; i++) {
			expect(els[i - 1]!.compareDocumentPosition(els[i]!) & Node.DOCUMENT_POSITION_FOLLOWING, `${order[i - 1]} before ${order[i]}`).toBeTruthy();
		}
		// Only the hero is inside the pager: the controls are always on screen.
		const pager = root.querySelector('[data-pager]')!;
		for (const q of ['h1.title', '[aria-label="Previous"]', '[data-up-next]', '[data-play-on]']) expect(pager.contains(root.querySelector(q))).toBe(false);
		// The transport: shuffle, previous, play/pause, next, repeat.
		const transport = root.querySelector('.transport')!;
		expect([...transport.querySelectorAll('button')].map((b) => b.getAttribute('aria-label'))).toEqual(['Shuffle: off', 'Previous', 'Pause', 'Next', 'Repeat: off']);
		// The bottom row: Play on, Lyrics and Queue (Wave 3.5), in a labelled group.
		const actions = r.getByRole('navigation', { name: 'Now playing actions' });
		expect([...actions.querySelectorAll(':scope > button')].map((b) => b.getAttribute('aria-label'))).toEqual(['Play on…', 'Lyrics', 'Queue']);
		// Gone: the segmented control, the volume row and button, the separate visualiser and
		// sleep buttons, car and alarms (all in ⋮ or Play on…), and the "Playing on" chip.
		for (const q of ['[data-layouts]', '.volrow', '[data-volume-button]', '[data-visualiser-button]', '[data-visualiser-style]', '[data-visualiser-toggle]', '[data-sleep-button]', '[data-car-link]', '[data-alarms-link]', '[data-lyrics-peek]', '.chip', 'input[type=range][max="100"]']) {
			expect(root.querySelector(q), q).toBeNull();
		}
		r.unmount();
	});

	test('the title is one line with one rating toggle beside it; artist · album and the quality badge under it', async () => {
		const r = render(NowPlaying);
		await tick();
		const row = r.container.querySelector('.title-row')!;
		expect(row.querySelector('h1')!.textContent).toBe('Harbour Lights');
		expect(row.querySelector('h1')!.getAttribute('title')).toBe('Harbour Lights');
		// One toggle, not five stars (the full rating is in ⋮ › This track).
		const toggle = row.querySelector<HTMLButtonElement>('[data-rating-toggle]')!;
		expect(toggle).not.toBeNull();
		expect(toggle.getAttribute('aria-label')).toBe('Love this track');
		expect(toggle.getAttribute('aria-pressed')).toBe('false');
		expect(row.querySelector('[data-rating-stars]')).toBeNull();
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

	test('the rating toggle loves the track (five stars), and again clears it', async () => {
		const r = render(NowPlaying);
		await tick();
		const toggle = r.container.querySelector<HTMLButtonElement>('[data-rating-toggle]')!;
		await fireEvent.click(toggle);
		await waitFor(() => expect(toggle.getAttribute('aria-pressed')).toBe('true'));
		await fireEvent.click(toggle);
		await waitFor(() => expect(toggle.getAttribute('aria-pressed')).toBe('false'));
		r.unmount();
	});

	test('a track rated in LMS shows as rated here, muted, until the user loves it (issue #14)', async () => {
		// Before: the toggle lit only for the user's own rating, so a track rated 4 or 5 stars in
		// LMS showed a blank star on phones while the wide layout's five stars showed the rating.
		const { ratings } = await import('$lib/state/ratings');
		const { routine } = await import('$lib/state/me');
		ratings.reset();
		const fetched = vi.spyOn(routine, 'ratings').mockResolvedValue([{ track: 11, mine: null, shared: 80, rating: 80 }]);
		const styled = vi.spyOn(routine, 'style').mockResolvedValue({ style: 'stars', count: 0 });
		const rated = vi.spyOn(routine, 'rate').mockImplementation(async (track, value) => ({ track, mine: value > 0 ? value : null, shared: 80, rating: value > 0 ? value : 80 }));
		const r = render(NowPlaying);
		const toggle = await waitFor(() => {
			const t = r.container.querySelector<HTMLButtonElement>('[data-rating-toggle].on');
			expect(t).not.toBeNull();
			return t!;
		});
		expect(toggle.classList.contains('shared')).toBe(true);
		expect(toggle.getAttribute('aria-pressed')).toBe('false');
		expect(toggle.getAttribute('aria-label')).toBe('Rated 4 of 5 on the server. Love this track');
		expect(toggle.getAttribute('title')).toBe('Rated 4 of 5 on the server. Tap to love it');
		// Loving it makes it the user's own (accent, pressed).
		await fireEvent.click(toggle);
		await waitFor(() => expect(toggle.getAttribute('aria-pressed')).toBe('true'));
		expect(toggle.classList.contains('shared')).toBe(false);
		expect(toggle.getAttribute('title')).toBe('Loved. Tap to clear your rating');
		// Clearing the user's own rating leaves the server's showing again.
		await fireEvent.click(toggle);
		await waitFor(() => expect(toggle.getAttribute('aria-pressed')).toBe('false'));
		expect(toggle.classList.contains('on')).toBe(true);
		expect(toggle.classList.contains('shared')).toBe(true);
		// Two stars on the server is not "rated" for the toggle: the full control in ⋮ shows it.
		r.unmount();
		ratings.reset();
		fetched.mockResolvedValue([{ track: 11, mine: null, shared: 40, rating: 40 }]);
		const r2 = render(NowPlaying);
		await tick();
		await waitFor(() => expect(fetched).toHaveBeenCalledTimes(2));
		await tick();
		expect(r2.container.querySelector('[data-rating-toggle].on')).toBeNull();
		r2.unmount();
		fetched.mockRestore();
		styled.mockRestore();
		rated.mockRestore();
		ratings.reset();
	});

	test('a stream has no rating and no lyrics page content; the controls stay', async () => {
		fake.app.now = status({ playlist_loop: [{ title: 'Radio Example', remote: 1 }] });
		const r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('[data-rating-stars], [data-rating-heart], [data-rating-toggle]')).toBeNull();
		expect(r.getByRole('button', { name: 'Pause' })).toBeTruthy();
		r.unmount();
	});

	test('no player chosen: says so, and a Choose a player button opens Play on…', async () => {
		fake.app.playerId = '';
		fake.app.now = null;
		const r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('h1')!.textContent).toBe('No player chosen');
		expect((r.getByRole('button', { name: 'Play' }) as HTMLButtonElement).disabled).toBe(true);
		await fireEvent.click(r.getByRole('button', { name: 'Choose a player' }));
		expect(r.container.querySelector('[role=dialog][aria-label="Play on"]')).not.toBeNull();
		expect(r.container.querySelector('[data-play-on]')!.getAttribute('aria-expanded')).toBe('true');
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

	test('⋮ opens a sheet of about four: This track, Sleep timer, Visualiser and Car mode', async () => {
		const r = render(NowPlaying);
		await tick();
		const more = r.getByRole('button', { name: 'More options' });
		more.focus();
		await fireEvent.click(more);
		const sheet = document.querySelector('dialog[open] [data-more-sheet]')!;
		expect(sheet).not.toBeNull();
		expect(sheet.textContent).toMatch(/This track.*Sleep timer.*Visualiser.*Car mode/);
		// Each meaning has one way in: Signal path is the badge's; Rooms and a player's Alarms
		// are in Play on….
		expect(sheet.textContent).not.toMatch(/Signal path|All rooms|Alarms/);
		expect(sheet.querySelectorAll('.row').length).toBe(4);
		expect(sheet.querySelector('[data-car-link]')!.getAttribute('href')).toBe('#/car');
		// Sleep timer: the sheet gives way to the timer's; closing it returns to ⋮.
		await fireEvent.click(sheet.querySelector('[data-sleep-button]')!);
		await waitFor(() => expect(document.querySelector('dialog[open] [data-sleep-sheet]')).not.toBeNull());
		expect(document.querySelector('[data-more-sheet]')).toBeNull();
		document.querySelector<HTMLDialogElement>('dialog[open]')!.close();
		await waitFor(() => expect(document.querySelector('[data-sleep-sheet]')).toBeNull());
		await waitFor(() => expect(document.activeElement).toBe(r.getByRole('button', { name: 'More options' })));
		r.unmount();
	});

	test('⋮ › This track has the full 1–5 rating and the credits', async () => {
		fake.handler.rpc = async (_p, cmd) => (cmd[0] === 'titles' ? { titles_loop: [] } : {});
		const r = render(NowPlaying);
		await tick();
		await fireEvent.click(r.getByRole('button', { name: 'More options' }));
		await fireEvent.click(document.querySelector('[data-more-sheet] [data-track-menu]')!);
		const rating = await waitFor(() => document.querySelector('dialog[open] [data-track-rating]')!);
		expect(rating.querySelector('[data-rating-stars]')!.getAttribute('aria-label')).toBe('Your rating');
		r.unmount();
	});

	test('⋮ › Visualiser opens the chooser; a remote player has no such row', async () => {
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
		nowPlaying.setVisualiser(true);
		flushSync();
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

describe('the visualiser, from ⋮', () => {
	async function chooser(c: HTMLElement) {
		await fireEvent.click(c.querySelector('[data-np-more]')!);
		await fireEvent.click(c.querySelector('[data-visualiser-style-row]')!);
		return document.querySelector('dialog[open] [data-style-sheet]')!;
	}

	test('choosing a style switches it on and goes to the cover; the style’s name is read out', async () => {
		nowPlaying.setLayout('lyrics');
		const r = render(NowPlaying);
		await tick();
		// The one snackbar says it (Wave 3.5: no toast of its own), in its live region.
		const toast = r.container.querySelector('[data-np-toast] [role=status]')!;
		expect(toast.getAttribute('aria-live')).toBe('polite');
		const sheet = await chooser(r.container);
		expect(sheet.getAttribute('aria-label')).toBe('Visualiser style');
		expect([...sheet.querySelectorAll('[data-style-choice]')].map((b) => b.textContent!.trim())).toEqual(['Bars', 'Scope', 'Glow', 'VU meters']);
		await fireEvent.click(sheet.querySelector('[data-style-choice="bars"]')!);
		expect(page(r.container)).toBe('art');
		expect(nowPlaying.visualiser).toBe(true);
		expect(toast.textContent).toBe('Bars');
		expect(r.container.querySelector('[data-visualiser="bars"]')).not.toBeNull();
		r.unmount();
	});

	test('Switch off is offered while it is on; the name shows briefly, then goes', async () => {
		vi.useFakeTimers();
		try {
			nowPlaying.setVisualiser(true);
			const r = render(NowPlaying);
			await tick();
			const sheet = await chooser(r.container);
			expect(sheet.querySelector('[data-style-choice="bars"]')!.getAttribute('aria-pressed')).toBe('true');
			await fireEvent.click(sheet.querySelector('[data-style-off]')!);
			expect(nowPlaying.visualiser).toBe(false);
			const toast = r.container.querySelector('[data-np-toast] [role=status]')!;
			expect(toast.textContent).toBe('Visualiser off');
			vi.advanceTimersByTime(2000);
			flushSync();
			expect(toast.textContent).toBe('');
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
		expect(r.container.querySelector('[data-np-toast] [role=status]')!.textContent).toBe('Scope');
		r.unmount();
	});

	test('off under reduced motion, and the row says why', async () => {
		reduceMotion = true;
		const r = render(NowPlaying);
		await tick();
		await fireEvent.click(r.container.querySelector('[data-np-more]')!);
		const row = r.container.querySelector<HTMLButtonElement>('[data-visualiser-style-row]')!;
		expect(row.disabled).toBe(true);
		expect(row.textContent).toMatch(/reduced motion/);
		r.unmount();
	});
});

describe('Play on (the bottom row), with the volume', () => {
	const open = async (c: HTMLElement) => {
		await fireEvent.click(c.querySelector('[data-play-on]')!);
		return c.querySelector<HTMLElement>('[role=dialog][aria-label="Play on"]')!;
	};

	test('names the player in use (the state), and opens the picker; Escape closes it', async () => {
		let r = render(NowPlaying);
		await tick();
		const btn = r.container.querySelector<HTMLButtonElement>('[data-play-on]')!;
		expect(btn.textContent).toContain('This device');
		expect(btn.getAttribute('aria-haspopup')).toBe('dialog');
		expect(btn.getAttribute('aria-expanded')).toBe('false');
		const picker = await open(r.container);
		expect(btn.getAttribute('aria-expanded')).toBe('true');
		expect(picker).not.toBeNull();
		await fireEvent.keyDown(picker, { key: 'Escape' });
		expect(r.container.querySelector('[role=dialog][aria-label="Play on"]')).toBeNull();
		r.unmount();
		fake.app.playerId = KITCHEN;
		r = render(NowPlaying);
		await tick();
		const remote = r.container.querySelector<HTMLButtonElement>('[data-play-on]')!;
		expect(remote.textContent).toContain('Kitchen');
		expect(remote.getAttribute('aria-label')).toBe('Play on… (now Kitchen)');
		r.unmount();
	});

	test('another player: the slider is labelled with its name, and sets that player’s volume', async () => {
		fake.app.playerId = KITCHEN;
		const r = render(NowPlaying);
		await tick();
		const picker = await open(r.container);
		const slider = picker.querySelector<HTMLInputElement>('[data-volume-slider] input[type=range]')!;
		expect(slider.getAttribute('aria-label')).toBe('Volume on Kitchen');
		expect(slider.value).toBe('40');
		await fireEvent.input(slider, { target: { value: '65' } });
		await waitFor(() => expect(sent().filter((c) => c.cmd[0] === 'mixer')).toEqual([{ player: KITCHEN, cmd: ['mixer', 'volume', 65] }]));
		r.unmount();
	});

	test('this browser is the player: it sets this browser’s volume (through LMS, as the slider always has)', async () => {
		const r = render(NowPlaying);
		await tick();
		const picker = await open(r.container);
		const slider = picker.querySelector<HTMLInputElement>('[data-volume-slider] input')!;
		expect(slider.getAttribute('aria-label')).toBe('Volume on this device');
		await fireEvent.input(slider, { target: { value: '20' } });
		await waitFor(() => expect(sent().filter((c) => c.cmd[0] === 'mixer')).toEqual([{ player: HERE, cmd: ['mixer', 'volume', 20] }]));
		r.unmount();
	});

	test('out-of-range or odd volumes are clamped before they are sent', async () => {
		fake.app.now = status({ 'mixer volume': -30 }); // LMS reports a muted player as negative
		const r = render(NowPlaying);
		await tick();
		const picker = await open(r.container);
		const slider = picker.querySelector<HTMLInputElement>('[data-volume-slider] input')!;
		expect(slider.value).toBe('30');
		await fireEvent.input(slider, { target: { value: '250' } });
		await waitFor(() => expect(sent().filter((c) => c.cmd[0] === 'mixer')).toEqual([{ player: HERE, cmd: ['mixer', 'volume', 100] }]));
		r.unmount();
	});
});

describe('the other actions', () => {
	test('Lyrics shows the pager’s lyrics page, and again the cover', async () => {
		const r = render(NowPlaying);
		await tick();
		const lyrics = r.container.querySelector<HTMLButtonElement>('[data-lyrics-button]')!;
		expect(lyrics.getAttribute('aria-pressed')).toBe('false');
		await fireEvent.click(lyrics);
		expect(page(r.container)).toBe('lyrics');
		expect(lyrics.getAttribute('aria-pressed')).toBe('true');
		await fireEvent.click(lyrics);
		expect(page(r.container)).toBe('art');
		r.unmount();
	});

	test('Queue opens the queue peek; so does a tap on Up next (the same sheet)', async () => {
		const r = render(NowPlaying);
		await tick();
		await fireEvent.click(r.container.querySelector('[data-queue-peek]')!);
		await waitFor(() => expect(document.querySelector('dialog[open] [data-full-queue]')).not.toBeNull());
		document.querySelector<HTMLDialogElement>('dialog[open]')!.close();
		await waitFor(() => expect(document.querySelector('[data-full-queue]')).toBeNull());
		await fireEvent.click(r.container.querySelector('[data-up-next] a')!);
		await waitFor(() => expect(document.querySelector('dialog[open] [data-full-queue]')).not.toBeNull());
		r.unmount();
	});

	test('Up next is one compact line, with no trailing control', async () => {
		const r = render(NowPlaying);
		await tick();
		const next = r.container.querySelector('[data-up-next]')!;
		expect(next.textContent).toContain('Up next');
		expect(next.textContent).toContain('Lantern Street');
		expect(next.textContent).toContain('— The Example Quartet');
		expect(next.querySelectorAll('button')).toHaveLength(0);
		expect(r.queryByRole('button', { name: 'Remove Lantern Street from the queue' })).toBeNull();
		r.unmount();
	});
});
