// Wave 1 on Now playing and the mini player: the quality badge and signal path, the sleep
// timer sheet, "Up next" (swipe or button to remove, with Undo), the ambient background,
// and the mini player's gestures with their keyboard and screen-reader equivalents.
import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { flushSync, tick } from 'svelte';
import type { PlayerStatus } from '$lib/lms/types';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const fake = await import('./helpers/fake-app.svelte');
const { default: NowPlaying } = await import('../src/routes/+page.svelte');
const { default: PlayerBar } = await import('$lib/components/PlayerBar.svelte');

const P = '02:00:00:00:00:01';
const OTHER = '02:00:00:00:00:02';

class NoObserver {
	observe() {}
	unobserve() {}
	disconnect() {}
}
let reduceMotion = false;
beforeAll(() => {
	vi.stubGlobal('IntersectionObserver', NoObserver);
	vi.stubGlobal('ResizeObserver', NoObserver);
	HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
		this.setAttribute('open', '');
	};
	HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
		this.removeAttribute('open');
		this.dispatchEvent(new Event('close'));
	};
	vi.stubGlobal('matchMedia', (q: string) => ({
		matches: q.includes('reduced-motion') ? reduceMotion : false,
		addEventListener() {},
		removeEventListener() {},
	}));
});
afterAll(() => vi.unstubAllGlobals());

function status(over: Partial<PlayerStatus> = {}): PlayerStatus {
	return {
		mode: 'play',
		time: 30,
		duration: 240,
		playlist_cur_index: 3,
		playlist_tracks: 8,
		'mixer volume': 50,
		playlist_loop: [
			{ id: 11, title: 'Current song', artist: 'Band', album: 'Record', album_id: 5, type: 'flc', samplesize: '24', samplerate: '96000', 'playlist index': 3 },
			{ id: 12, title: 'Next song', artist: 'Other band', 'playlist index': 4, duration: 200 },
		],
		...over,
	};
}

let queueLen = 8;
beforeEach(() => {
	fake.calls.length = 0;
	fake.app.sleepCalls.length = 0;
	fake.app.players = [
		{ playerid: P, name: 'Test player' },
		{ playerid: OTHER, name: 'Kitchen', connected: 1 },
	];
	fake.app.playerId = P;
	fake.app.now = status();
	reduceMotion = false;
	queueLen = 8;
	fake.handler.rpc = async (_p, cmd) => {
		if (cmd[0] === 'status' && cmd[1] === '-') return { playlist_tracks: queueLen };
		if (cmd[0] === 'playlistcontrol') queueLen++;
		if (cmd[0] === 'playlist' && cmd[1] === 'delete') queueLen--;
		return {};
	};
	location.hash = '#/queue';
});

const sent = () => fake.calls.map((c) => c.cmd).filter((c) => c[0] !== 'status' && c[0] !== 'players');

function pointer(el: Element, type: string, init: PointerEventInit & { timeStamp?: number }) {
	const e = new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, pointerType: 'touch', ...init });
	el.dispatchEvent(e);
	flushSync();
}

describe('Now playing', () => {
	test('the quality badge reads FLAC 24/96 · Hi-Res and opens the signal path', async () => {
		const r = render(NowPlaying);
		await tick();
		const badge = r.container.querySelector<HTMLButtonElement>('[data-quality-badge]')!;
		expect(badge.textContent).toBe('FLAC 24/96 · Hi-Res');
		expect(badge.getAttribute('aria-label')).toMatch(/signal path/i);
		await fireEvent.click(badge);
		const path = await waitFor(() => r.container.querySelector('[data-signal-path]')!);
		expect(path.textContent).toContain('Source');
		expect(path.textContent).toContain('FLAC 24/96 · Hi-Res · lossless');
		// Another player is controlled: only what is known is claimed.
		expect(path.textContent).toContain('On Test player');
		r.unmount();
	});

	test('a lossy track shows codec and bit rate, without Hi-Res', async () => {
		fake.app.now = status({ playlist_loop: [{ id: 11, title: 'Song', type: 'mp3', bitrate: '320kbps', samplerate: '44100' }] });
		const r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('[data-quality-badge]')!.textContent).toBe('MP3 320');
		r.unmount();
	});

	test('the moon opens the sleep sheet; a length is set on the chosen players', async () => {
		const r = render(NowPlaying);
		await tick();
		const moon = r.container.querySelector<HTMLButtonElement>('[data-sleep-button]')!;
		expect(moon.getAttribute('aria-label')).toBe('Sleep timer');
		await fireEvent.click(moon);
		const sheet = await waitFor(() => r.container.querySelector('[data-sleep-sheet]')!);
		// Another connected player: "All players" is offered.
		const all = [...sheet.querySelectorAll<HTMLInputElement>('input[type=radio]')].find((i) => i.value === 'all')!;
		await fireEvent.click(all);
		await fireEvent.click([...sheet.querySelectorAll('button')].find((b) => b.textContent === '30 min')!);
		expect(fake.app.sleepCalls).toEqual([{ choice: { kind: 'minutes', minutes: 30 }, scope: 'all' }]);
		r.unmount();
	});

	test('end of track/album/queue are offered for tracks, not for a stream', async () => {
		let r = render(NowPlaying);
		await fireEvent.click(r.container.querySelector('[data-sleep-button]')!);
		let sheet = await waitFor(() => r.container.querySelector('[data-sleep-sheet]')!);
		const album = [...sheet.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === 'End of this album')!;
		expect(album.disabled).toBe(false);
		await fireEvent.click(album);
		expect(fake.app.sleepCalls.at(-1)).toEqual({ choice: { kind: 'album' }, scope: 'player' });
		r.unmount();
		fake.app.now = status({ duration: 0, playlist_loop: [{ title: 'Radio', remote: 1 }] });
		r = render(NowPlaying);
		await fireEvent.click(r.container.querySelector('[data-sleep-button]')!);
		sheet = await waitFor(() => r.container.querySelector('[data-sleep-sheet]')!);
		for (const label of ['End of this track', 'End of this album', 'End of the queue']) {
			expect([...sheet.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === label)!.disabled).toBe(true);
		}
		r.unmount();
	});

	test('a custom length is checked before it is sent', async () => {
		const r = render(NowPlaying);
		await fireEvent.click(r.container.querySelector('[data-sleep-button]')!);
		const sheet = await waitFor(() => r.container.querySelector('[data-sleep-sheet]')!);
		const input = sheet.querySelector<HTMLInputElement>('#sleep-custom')!;
		for (const bad of ['0', '5000', '-3']) {
			await fireEvent.input(input, { target: { value: bad } });
			await fireEvent.submit(sheet.querySelector('form')!);
			expect(sheet.querySelector('[role=alert]')?.textContent).toMatch(/whole number of minutes/);
		}
		expect(fake.app.sleepCalls).toEqual([]);
		await fireEvent.input(input, { target: { value: '20' } });
		await fireEvent.submit(sheet.querySelector('form')!);
		expect(fake.app.sleepCalls).toEqual([{ choice: { kind: 'minutes', minutes: 20 }, scope: 'player' }]);
		r.unmount();
	});

	test('a running timer shows the time left, and one tap turns it off', async () => {
		fake.app.now = status({ will_sleep_in: 1620 });
		const r = render(NowPlaying);
		await tick();
		const moon = r.container.querySelector('[data-sleep-button]')!;
		expect(moon.getAttribute('aria-label')).toBe('Sleep timer: stops in 27 min');
		expect(moon.textContent).toContain('27 min');
		await fireEvent.click(r.container.querySelector('[data-sleep-cancel]')!);
		expect(fake.app.sleepCalls).toEqual([{ choice: 'off', scope: null }]);
		r.unmount();
	});

	test('Up next shows the next track; its remove button takes it out, and Undo puts it back', async () => {
		const r = render(NowPlaying);
		await tick();
		const row = r.container.querySelector('[data-up-next]')!;
		expect(row.textContent).toContain('Up next');
		expect(row.textContent).toContain('Next song');
		expect(row.querySelector('a')!.getAttribute('href')).toBe('#/queue');
		await fireEvent.click(r.getByRole('button', { name: 'Remove Next song from the queue' }));
		await waitFor(() => expect(sent()).toEqual([['playlist', 'delete', 4]]));
		await waitFor(() => expect(r.container.querySelector('[data-up-next]')).toBeNull()); // hidden at once
		const undo = await waitFor(() => r.getByRole('button', { name: 'Undo' }));
		fake.calls.length = 0;
		await fireEvent.click(undo);
		await waitFor(() => expect(sent()).toEqual([['playlistcontrol', 'cmd:add', 'track_id:12'], ['playlist', 'move', 7, 4]]));
		r.unmount();
	});

	test('Up next: swipe sideways to remove; Delete on the focused row does the same', async () => {
		let r = render(NowPlaying);
		await tick();
		const link = r.container.querySelector('[data-up-next] a')!;
		pointer(link, 'pointerdown', { clientX: 200, clientY: 10 });
		pointer(link, 'pointermove', { clientX: 150, clientY: 12 });
		pointer(link, 'pointermove', { clientX: 60, clientY: 14 });
		pointer(link, 'pointerup', { clientX: 60, clientY: 14 });
		await waitFor(() => expect(sent()).toEqual([['playlist', 'delete', 4]]));
		r.unmount();
		fake.calls.length = 0;
		r = render(NowPlaying);
		await tick();
		await fireEvent.keyDown(r.container.querySelector('[data-up-next] a')!, { key: 'Delete' });
		await waitFor(() => expect(sent()).toEqual([['playlist', 'delete', 4]]));
		r.unmount();
	});

	test('a short or vertical swipe on Up next removes nothing', async () => {
		const r = render(NowPlaying);
		await tick();
		const link = r.container.querySelector('[data-up-next] a')!;
		pointer(link, 'pointerdown', { clientX: 200, clientY: 10 });
		pointer(link, 'pointermove', { clientX: 160, clientY: 12 });
		pointer(link, 'pointerup', { clientX: 160, clientY: 12 });
		pointer(link, 'pointerdown', { clientX: 200, clientY: 10 });
		pointer(link, 'pointermove', { clientX: 198, clientY: 120 });
		pointer(link, 'pointerup', { clientX: 198, clientY: 120 });
		await tick();
		expect(sent()).toEqual([]);
		r.unmount();
	});

	test('no Up next under "repeat this track", or at the end of the queue', async () => {
		fake.app.now = status({ 'playlist repeat': 1 });
		let r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('[data-up-next]')).toBeNull();
		r.unmount();
		fake.app.now = status({ playlist_loop: [status().playlist_loop![0]!] });
		r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('[data-up-next]')).toBeNull();
		r.unmount();
	});

	test('the ambient background drifts, and holds still under "reduce motion"', async () => {
		let r = render(NowPlaying);
		await tick();
		let amb = r.container.querySelector('[data-ambient]')!;
		expect(amb.getAttribute('aria-hidden')).toBe('true');
		expect(amb.classList.contains('still')).toBe(false);
		r.unmount();
		reduceMotion = true;
		r = render(NowPlaying);
		await tick();
		amb = r.container.querySelector('[data-ambient]')!;
		expect(amb.classList.contains('still')).toBe(true);
		r.unmount();
	});
});

describe('mini player gestures', () => {
	const mini = (r: { container: HTMLElement }) => r.container.querySelector<HTMLElement>('[data-mini-player]')!;

	function drag(el: HTMLElement, dx: number, dy: number) {
		pointer(el, 'pointerdown', { clientX: 100, clientY: 100 });
		for (let i = 1; i <= 4; i++) pointer(el, 'pointermove', { clientX: 100 + (dx * i) / 4, clientY: 100 + (dy * i) / 4 });
		pointer(el, 'pointerup', { clientX: 100 + dx, clientY: 100 + dy });
	}

	test('swipe left for next, right for previous', async () => {
		const r = render(PlayerBar);
		await tick();
		drag(mini(r), -120, 4);
		await waitFor(() => expect(sent()).toEqual([['playlist', 'index', '+1']]));
		drag(mini(r), 120, -4);
		await waitFor(() => expect(sent()).toEqual([['playlist', 'index', '+1'], ['playlist', 'index', '-1']]));
		// The click that ends a swipe doesn't also open Now playing.
		const click = new MouseEvent('click', { bubbles: true, cancelable: true });
		mini(r).dispatchEvent(click);
		expect(click.defaultPrevented).toBe(true);
		r.unmount();
	});

	test('swipe up opens Now playing', async () => {
		const r = render(PlayerBar);
		await tick();
		drag(mini(r), 3, -80);
		expect(location.hash).toBe('#/');
		r.unmount();
	});

	test('long-press opens Play on…', async () => {
		vi.useFakeTimers();
		try {
			const r = render(PlayerBar);
			await tick();
			pointer(mini(r), 'pointerdown', { clientX: 50, clientY: 50 });
			vi.advanceTimersByTime(600);
			flushSync();
			expect(r.container.querySelector('[role=dialog][aria-label="Play on"]')).not.toBeNull();
			pointer(mini(r), 'pointerup', { clientX: 50, clientY: 50 });
			// …and the lift doesn't also open Now playing.
			const click = new MouseEvent('click', { bubbles: true, cancelable: true });
			mini(r).dispatchEvent(click);
			expect(click.defaultPrevented).toBe(true);
			r.unmount();
		} finally {
			vi.useRealTimers();
		}
	});

	test('a tap is left to the link (opens Now playing)', async () => {
		const r = render(PlayerBar);
		await tick();
		pointer(mini(r), 'pointerdown', { clientX: 50, clientY: 50 });
		pointer(mini(r), 'pointerup', { clientX: 51, clientY: 50 });
		const click = new MouseEvent('click', { bubbles: true, cancelable: true });
		mini(r).dispatchEvent(click);
		expect(click.defaultPrevented).toBe(false);
		expect(mini(r).getAttribute('href')).toBe('#/');
		expect(sent()).toEqual([]);
		r.unmount();
	});

	test('every gesture has a keyboard shortcut and a button', async () => {
		const r = render(PlayerBar);
		await tick();
		await fireEvent.keyDown(mini(r), { key: 'ArrowRight', altKey: true });
		await fireEvent.keyDown(mini(r), { key: 'ArrowLeft', altKey: true });
		await waitFor(() => expect(sent()).toEqual([['playlist', 'index', '+1'], ['playlist', 'index', '-1']]));
		await fireEvent.keyDown(mini(r), { key: 'F10', shiftKey: true });
		expect(r.container.querySelector('[role=dialog][aria-label="Play on"]')).not.toBeNull();
		// Buttons for screen readers (kept in the page on phones, only visually hidden).
		expect(r.getByRole('button', { name: 'Previous' })).toBeTruthy();
		expect(r.getByRole('button', { name: 'Next' })).toBeTruthy();
		expect(r.container.querySelector('[data-output-button]')).not.toBeNull();
		// The gestures are described to screen readers.
		const hint = document.getElementById(mini(r).getAttribute('aria-describedby')!)!;
		expect(hint.textContent).toMatch(/Swipe left or right/);
		r.unmount();
	});

	test('a right-click (context menu) opens Play on… rather than the browser menu', async () => {
		const r = render(PlayerBar);
		await tick();
		const e = new MouseEvent('contextmenu', { bubbles: true, cancelable: true });
		mini(r).dispatchEvent(e);
		flushSync();
		expect(e.defaultPrevented).toBe(true);
		expect(r.container.querySelector('[role=dialog][aria-label="Play on"]')).not.toBeNull();
		r.unmount();
	});

	test('a small moon shows while a sleep timer runs', async () => {
		fake.app.now = status({ will_sleep_in: 300 });
		const r = render(PlayerBar);
		await tick();
		expect(r.container.querySelector('.moon')).not.toBeNull();
		expect(mini(r).getAttribute('aria-label')).toBe('Now playing: Current song. Sleep timer: 5 min');
		r.unmount();
		fake.app.now = status();
		const r2 = render(PlayerBar);
		await tick();
		expect(r2.container.querySelector('.moon')).toBeNull();
		r2.unmount();
	});
});
