// The queue for Wave 2 (§3.4, §3.5): sections from this device's Up next (flat when they can't
// be vouched for), swipe right to play next with the menu and N key as alternatives, Clear
// upcoming (from the end, checked; Clear all for a long queue), gapless (said in track info,
// not between rows, since Wave 3.5), the queue peek on Now playing, and the shuffle button's
// three modes.
import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { flushSync, tick } from 'svelte';
import type { PlayerStatus, Track } from '$lib/lms/types';
import { removeAt, reorder } from '$lib/lms/queue-edit';
import { upNextTracker } from '$lib/queue/upnext.svelte';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const fake = await import('./helpers/fake-app.svelte');
const { default: QueuePage } = await import('../src/routes/queue/+page.svelte');
const { default: QueuePeek } = await import('$lib/components/QueuePeek.svelte');
const { default: ShuffleButton } = await import('$lib/components/ShuffleButton.svelte');
const { default: NowPlaying } = await import('../src/routes/+page.svelte');
const { default: NowPlayingInfo } = await import('$lib/components/NowPlayingInfo.svelte');
const { choose, menuItems } = await import('./helpers/menu');

const P = '02:00:00:00:00:01';
let queue: Track[] = [];
let cur = 1;

const track = (n: number, over: Partial<Track> = {}): Track => ({ id: n, title: `Song ${n}`, artist: 'Invented', duration: 180, ...over });

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
	vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (this: Element) {
		const li = this.closest('li[data-index]');
		const i = li ? Number(li.getAttribute('data-index')) : 0;
		return { top: i * 50, height: 50, width: 360, left: 0, right: 360, bottom: i * 50 + 50, x: 0, y: i * 50, toJSON() {} } as DOMRect;
	});
});
afterAll(() => vi.unstubAllGlobals());

function nowStatus(): PlayerStatus {
	return { mode: 'play', playlist_cur_index: cur, playlist_tracks: queue.length, 'playlist shuffle': 0, playlist_loop: [{ ...queue[cur], 'playlist index': cur }] };
}

beforeEach(() => {
	fake.calls.length = 0;
	queue = [1, 2, 3, 4, 5, 6].map((n) => track(n));
	cur = 1;
	upNextTracker.reset();
	fake.app.players = [{ playerid: P, name: 'Test player' }];
	fake.app.playerId = P;
	fake.app.now = nowStatus();
	fake.handler.rpc = async (_p, cmd) => {
		const key = cmd.slice(0, 2).join(' ');
		if (cmd[0] === 'status' && cmd[1] === '-') return nowStatus();
		if (cmd[0] === 'status') {
			const from = Number(cmd[1]);
			const loop = queue.slice(from, from + Number(cmd[2])).map((t, i) => ({ ...t, 'playlist index': from + i }));
			return { playlist_loop: loop, playlist_tracks: queue.length, playlist_cur_index: cur };
		}
		if (key === 'playlist move') {
			const [from, to] = [Number(cmd[2]), Number(cmd[3])];
			queue = reorder(queue, from, to);
			if (from === cur) cur = to;
			else if (from < cur && to >= cur) cur--;
			else if (from > cur && to <= cur) cur++;
		}
		if (key === 'playlist delete') queue = removeAt(queue, Number(cmd[2]));
		if (key === 'playlist clear') queue = [];
		return {};
	};
});

const titles = (c: HTMLElement) => [...c.querySelectorAll('li[data-index] .title')].map((e) => e.textContent);
const sent = () => fake.calls.filter((c) => c.cmd[0] !== 'status').map((c) => c.cmd);
const settle = async () => {
	for (let i = 0; i < 6; i++) {
		await tick();
		await new Promise((r) => setTimeout(r, 0));
	}
};
function pointer(el: Element, type: string, init: PointerEventInit) {
	el.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, ...init }));
	flushSync();
}
/** As if this device had sent Play next of `n` tracks and LMS had put them after the current one. */
function playedNext(n: number) {
	upNextTracker.observe(P, { count: queue.length, cur, shuffle: 0 }, 0);
	upNextTracker.expectInsert(P, { count: queue.length, cur, shuffle: 0 }, 1);
	const added = Array.from({ length: n }, (_, i) => track(90 + i, { title: `Added ${i + 1}` }));
	queue = [...queue.slice(0, cur + 1), ...added, ...queue.slice(cur + 1)];
	upNextTracker.observe(P, { count: queue.length, cur, shuffle: 0 }, 2);
	fake.app.now = nowStatus();
}

async function setup() {
	const r = render(QueuePage);
	await waitFor(() => expect(titles(r.container).length).toBe(queue.length));
	await settle();
	fake.calls.length = 0;
	return r;
}
const groups = (c: HTMLElement) => [...c.querySelectorAll('h2.section')].map((h) => h.textContent);

test('with Play next from this device: Played, Now playing, Up next and Then', async () => {
	playedNext(2);
	const r = await setup();
	expect(groups(r.container)).toEqual(['Played', 'Now playing', 'Up next', 'Then']);
	const next = r.container.querySelector('ol[data-group="next"]')!;
	expect(next.getAttribute('aria-labelledby')).toBe('q-next');
	expect([...next.querySelectorAll('.title')].map((e) => e.textContent)).toEqual(['Added 1', 'Added 2']);
	// Numbered as LMS numbers them.
	expect(next.getAttribute('start')).toBe('3');
	expect(r.container.querySelector('[data-sections]')?.getAttribute('data-sections')).toBe('on');
});

test('without (or after an unexplained change) the queue shows flat', async () => {
	const r = await setup();
	expect(groups(r.container)).toEqual([]);
	expect(r.container.querySelectorAll('ol').length).toBe(1);
	r.unmount();
	playedNext(2);
	// Another app reordered the Up next tracks: the ids no longer match.
	const s = await setup();
	expect(groups(s.container)).toContain('Up next');
	s.unmount();
	queue = [queue[0]!, queue[1]!, queue[3]!, queue[2]!, ...queue.slice(4)];
	const t = await setup();
	expect(groups(t.container)).toEqual([]);
});

test('swipe right plays a track next: moved to just after the current one, joining Up next', async () => {
	const r = await setup();
	const main = r.getByRole('button', { name: 'Play Song 5' });
	pointer(main, 'pointerdown', { clientX: 50, clientY: 210, pointerType: 'touch' });
	pointer(main, 'pointermove', { clientX: 120, clientY: 212, pointerType: 'touch' });
	pointer(main, 'pointermove', { clientX: 220, clientY: 212, pointerType: 'touch' });
	pointer(main, 'pointerup', { clientX: 220, clientY: 212, pointerType: 'touch' });
	await settle();
	expect(sent()).toEqual([['playlist', 'move', 4, 2]]);
	expect(titles(r.container)).toEqual(['Song 1', 'Song 2', 'Song 5', 'Song 3', 'Song 4', 'Song 6']);
	expect(r.getByRole('status').textContent).toContain('Song 5 will play next');
	await waitFor(() => expect(groups(r.container)).toEqual(['Played', 'Now playing', 'Up next', 'Then']));
	expect([...r.container.querySelectorAll('ol[data-group="next"] .title')].map((e) => e.textContent)).toEqual(['Song 5']);
});

test('every swipe has alternatives: the row menu and keys (N, Delete)', async () => {
	const r = await setup();
	// The menu: Play next, the moves, and Remove (last).
	await fireEvent.click(r.getByRole('button', { name: 'More options for Song 6' }));
	await settle();
	const items = r.getAllByRole('menuitem');
	expect(items.map((i) => i.firstElementChild?.nextElementSibling?.textContent)).toEqual(['Play next', 'Move up', 'Move down', 'Remove from queue']);
	expect(items[0]!.getAttribute('aria-keyshortcuts')).toBe('N');
	await fireEvent.click(items[0]!);
	await settle();
	expect(sent()).toEqual([['playlist', 'move', 5, 2]]);
	fake.calls.length = 0;
	// N on a focused row.
	const row = r.getByRole('button', { name: 'Play Song 4' });
	row.focus();
	await fireEvent.keyDown(row, { key: 'n' });
	await settle();
	expect(sent()).toEqual([['playlist', 'move', 4, 2]]);
	fake.calls.length = 0;
	// Delete on a focused row removes it (checked first).
	// (The queue is now 1, 2, 4, 6, 3, 5.)
	await fireEvent.keyDown(r.getByRole('button', { name: 'Play Song 3' }), { key: 'Delete' });
	await settle();
	expect(sent()).toEqual([['playlist', 'delete', 4]]);
	// The current track: nothing to move.
	fake.calls.length = 0;
	await fireEvent.keyDown(r.getByRole('button', { name: 'Play Song 2' }), { key: 'N' });
	await settle();
	expect(sent()).toEqual([]);
});

test('Clear upcoming asks, then deletes from the end, each checked', async () => {
	const r = await setup();
	await choose('Queue options', 'Clear upcoming…', r.container);
	await settle();
	const dialog = r.container.ownerDocument.querySelector('[data-confirm-dialog]')!;
	expect(dialog.textContent).toContain('Clear upcoming?');
	expect(dialog.textContent).toContain('Take out the 4 tracks after the one playing now?');
	// Focus starts on Cancel.
	expect(document.activeElement?.hasAttribute('data-confirm-cancel')).toBe(true);
	await fireEvent.click(dialog.querySelector('[data-confirm-ok]')!);
	await settle();
	expect(sent()).toEqual([
		['playlist', 'delete', 5],
		['playlist', 'delete', 4],
		['playlist', 'delete', 3],
		['playlist', 'delete', 2],
	]);
	const checks = fake.calls.filter((c) => c.cmd[0] === 'status' && c.cmd[2] === 1).map((c) => c.cmd[1]);
	expect(checks).toEqual([5, 4, 3, 2]);
	await waitFor(() => expect(titles(r.container)).toEqual(['Song 1', 'Song 2']));
	// Nothing after the current track: no Clear upcoming any more.
	expect(await menuItems('Queue options', r.container)).not.toContain('Clear upcoming…');
});

test('Clear upcoming stops when the queue changed meanwhile, and says so', async () => {
	const r = await setup();
	queue = [...queue.slice(0, 3), track(77), ...queue.slice(4)];
	await choose('Queue options', 'Clear upcoming…', r.container);
	await settle();
	await fireEvent.click(document.querySelector('[data-confirm-ok]')!);
	await settle();
	expect(sent()).toEqual([
		['playlist', 'delete', 5],
		['playlist', 'delete', 4],
	]);
	await waitFor(() => expect(r.getByRole('alert').textContent).toContain('stopped after removing 2 tracks'));
});

test('Cancel clears nothing', async () => {
	const r = await setup();
	await choose('Queue options', 'Clear upcoming…', r.container);
	await settle();
	await fireEvent.click(document.querySelector('[data-confirm-cancel]')!);
	await settle();
	expect(sent()).toEqual([]);
});

test('a long queue offers Clear all instead', async () => {
	queue = Array.from({ length: 150 }, (_, i) => track(i + 1));
	const r = await setup();
	await choose('Queue options', 'Clear upcoming…', r.container);
	await settle();
	const dialog = document.querySelector('[data-confirm-dialog]')!;
	expect(dialog.textContent).toContain('Clear the whole queue?');
	expect(dialog.querySelector('[data-confirm-ok]')?.textContent).toBe('Clear all');
	await fireEvent.click(dialog.querySelector('[data-confirm-ok]')!);
	await settle();
	expect(sent()).toEqual([['playlist', 'clear']]);
});

test('gapless is said in track info only, never between the queue’s rows', async () => {
	queue = [
		track(1, { album_id: 7, tracknum: 1 }),
		track(2, { album_id: 7, tracknum: 2 }),
		track(3, { album_id: 7, tracknum: 3 }),
		track(4, { album_id: 8, tracknum: 4 }),
		track(5, { album_id: 7, tracknum: 4 }),
	];
	cur = 1;
	fake.app.now = nowStatus();
	const r = await setup();
	expect(r.container.querySelector('[data-gapless]')).toBeNull();
	r.unmount();
	// Track info: Song 2 runs straight on into Song 3.
	const info = render(NowPlayingInfo);
	await waitFor(() => expect(info.container.querySelector('[data-gapless]')?.textContent).toBe('Gapless: runs straight on into the next track, from the same album.'));
	info.unmount();
	// Song 3 then Song 4 (another album): not gapless.
	cur = 2;
	fake.app.now = nowStatus();
	const other = render(NowPlayingInfo);
	await settle();
	expect(other.container.querySelector('[data-gapless]')).toBeNull();
	other.unmount();
});

test('the queue peek shows sections from the current track and plays a tapped one', async () => {
	playedNext(1);
	let closed = 0;
	const r = render(QueuePeek, { onclose: () => closed++ });
	await waitFor(() => expect(r.container.ownerDocument.querySelector('[data-peek="now"]')).not.toBeNull());
	const doc = r.container.ownerDocument;
	expect(fake.calls.find((c) => c.cmd[0] === 'status')?.cmd).toEqual(['status', 1, 30, 'tags:alcdeJKjt']);
	expect([...doc.querySelectorAll('[data-peek] ')].map((o) => o.getAttribute('data-peek'))).toEqual(['now', 'next', 'then']);
	expect(doc.querySelector('[data-peek="next"]')?.textContent).toContain('Added 1');
	await fireEvent.click(doc.querySelector('[data-peek="then"] button')!);
	expect(sent().at(-1)).toEqual(['playlist', 'index', 3]);
	expect(closed).toBe(1);
	expect(doc.querySelector('[data-full-queue]')?.getAttribute('href')).toBe('#/queue');
});

test('Now playing: the Queue button and a swipe up over the cover open the peek', async () => {
	const r = render(NowPlaying);
	const doc = r.container.ownerDocument;
	await fireEvent.click(r.container.querySelector('[data-queue-peek]')!);
	await waitFor(() => expect(doc.querySelector('[data-peek="now"]')).not.toBeNull());
	await fireEvent.click(doc.querySelector('dialog [aria-label="Close"]')!);
	await settle();
	expect(doc.querySelector('[data-peek="now"]')).toBeNull();
	const art = r.container.querySelector('[data-pager]')!;
	pointer(art, 'pointerdown', { clientX: 100, clientY: 300, pointerType: 'touch' });
	pointer(art, 'pointerup', { clientX: 105, clientY: 200, pointerType: 'touch' });
	await waitFor(() => expect(doc.querySelector('[data-peek="now"]')).not.toBeNull());
	r.unmount();
	// A sideways or short movement, or a mouse, doesn't.
	const s = render(NowPlaying);
	const art2 = s.container.querySelector('[data-pager]')!;
	pointer(art2, 'pointerdown', { clientX: 100, clientY: 300, pointerType: 'touch' });
	pointer(art2, 'pointerup', { clientX: 250, clientY: 260, pointerType: 'touch' });
	pointer(art2, 'pointerdown', { clientX: 100, clientY: 300, pointerType: 'mouse' });
	pointer(art2, 'pointerup', { clientX: 100, clientY: 100, pointerType: 'mouse' });
	await settle();
	expect(s.container.ownerDocument.querySelector('[data-peek="now"]')).toBeNull();
});

test('shuffle cycles Off → Tracks → Albums, and its name says which', async () => {
	const changes: number[] = [];
	const r = render(ShuffleButton, { value: 0, onchange: (m: number) => changes.push(m) });
	const btn = r.container.querySelector<HTMLButtonElement>('[data-shuffle]')!;
	expect(btn.getAttribute('aria-label')).toBe('Shuffle: off');
	await fireEvent.click(btn);
	expect(changes).toEqual([1]);
	await r.rerender({ value: 1 });
	expect(btn.getAttribute('aria-label')).toBe('Shuffle: tracks');
	await fireEvent.click(btn);
	await r.rerender({ value: '2' });
	expect(btn.getAttribute('aria-label')).toBe('Shuffle: albums');
	expect(btn.textContent).toContain('A');
	await fireEvent.click(btn);
	expect(changes).toEqual([1, 2, 0]);
	// How to get the choices is described for screen readers.
	expect(document.getElementById(btn.getAttribute('aria-describedby')!)?.textContent).toContain('Long press');
});

test('shuffle: the down arrow or a long press offers the three as radio items', async () => {
	vi.useFakeTimers();
	try {
		const changes: number[] = [];
		const r = render(ShuffleButton, { value: 2, onchange: (m: number) => changes.push(m) });
		const btn = r.container.querySelector<HTMLButtonElement>('[data-shuffle]')!;
		await fireEvent.keyDown(btn, { key: 'ArrowDown' });
		await vi.advanceTimersByTimeAsync(0);
		const items = () => [...r.container.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')];
		expect(items().map((i) => [i.textContent?.replace('✓', '').trim(), i.getAttribute('aria-checked')])).toEqual([
			['Shuffle off', 'false'],
			['Shuffle tracks', 'false'],
			['Shuffle albums', 'true'],
		]);
		expect(document.activeElement).toBe(items()[2]);
		await fireEvent.keyDown(items()[2]!, { key: 'Escape' });
		expect(document.activeElement).toBe(btn);
		// Long press: the menu, and the press's click doesn't also cycle.
		await fireEvent.pointerDown(btn, { pointerType: 'touch', pointerId: 4 });
		await vi.advanceTimersByTimeAsync(600);
		expect(items()).toHaveLength(3);
		await fireEvent.pointerUp(btn, { pointerType: 'touch', pointerId: 4 });
		await fireEvent.click(btn);
		expect(changes).toEqual([]);
		await vi.advanceTimersByTimeAsync(0);
		await fireEvent.click(r.container.querySelector('[data-shuffle-choice="0"]')!);
		expect(changes).toEqual([0]);
		// A quick tap cycles.
		await fireEvent.pointerDown(btn, { pointerType: 'touch', pointerId: 5 });
		await vi.advanceTimersByTimeAsync(100);
		await fireEvent.pointerUp(btn, { pointerType: 'touch', pointerId: 5 });
		await fireEvent.click(btn);
		expect(changes).toEqual([0, 0]);
	} finally {
		vi.useRealTimers();
	}
});

test('Now playing sends the shuffle mode chosen', async () => {
	fake.app.now = { ...nowStatus(), 'playlist shuffle': 1 };
	const r = render(NowPlaying);
	const btn = r.container.querySelector<HTMLButtonElement>('[data-shuffle]')!;
	expect(btn.getAttribute('aria-label')).toBe('Shuffle: tracks');
	await fireEvent.click(btn);
	await settle();
	expect(sent()).toContainEqual(['playlist', 'shuffle', 2]);
});
