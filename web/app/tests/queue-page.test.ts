// The queue page: drag to reorder, swipe to remove, Undo, keyboard moves, and that every edit
// shows at once, is sent as an allowed command, and is put back if LMS refuses it.
import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { flushSync, tick } from 'svelte';
import type { Track } from '$lib/lms/types';
import { removeAt, reorder } from '$lib/lms/queue-edit';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const fake = await import('./helpers/fake-app.svelte');
const { default: QueuePage } = await import('../src/routes/queue/+page.svelte');

const P = '02:00:00:00:00:01';
let queue: Track[] = [];
/** Commands (by their first two words) that LMS should refuse. */
let refuse: string[] = [];

function track(n: number, id: number = n): Track {
	return { id, title: `Song ${n}`, artist: 'Artist', duration: 180 };
}

beforeAll(() => {
	globalThis.IntersectionObserver = class {
		observe() {}
		disconnect() {}
		unobserve() {}
		takeRecords() {
			return [];
		}
		root = null;
		rootMargin = '';
		thresholds = [];
	} as unknown as typeof IntersectionObserver;
	// Rows are 50px tall, stacked from the top; each 360px wide.
	vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (this: Element) {
		const li = this.closest('li[data-index]');
		const i = li ? Number(li.getAttribute('data-index')) : 0;
		return { top: i * 50, height: 50, width: 360, left: 0, right: 360, bottom: i * 50 + 50, x: 0, y: i * 50, toJSON() {} } as DOMRect;
	});
});

beforeEach(() => {
	fake.calls.length = 0;
	refuse = [];
	queue = [track(1), track(2), track(3), track(4)];
	fake.app.players = [{ playerid: P, name: 'Test player' }];
	fake.app.playerId = P;
	fake.app.now = null;
	fake.handler.rpc = async (_player, cmd) => {
		const key = cmd.slice(0, 2).join(' ');
		if (refuse.some((r) => key.startsWith(r))) throw new Error('refused');
		if (cmd[0] === 'status' && cmd[1] === '-') return { playlist_tracks: queue.length };
		if (cmd[0] === 'status') {
			const from = Number(cmd[1]);
			const loop = queue.slice(from, from + Number(cmd[2])).map((t, i) => ({ ...t, 'playlist index': from + i }));
			return { playlist_loop: loop, playlist_tracks: queue.length };
		}
		// A tiny LMS: apply the edits to its queue.
		if (key === 'playlist move') queue = reorder(queue, Number(cmd[2]), Number(cmd[3]));
		if (key === 'playlist delete') queue = removeAt(queue, Number(cmd[2]));
		if (key === 'playlistcontrol cmd:add') queue = [...queue, track(Number(String(cmd[2]).split(':')[1]))];
		return {};
	};
});

const titles = (c: HTMLElement) => [...c.querySelectorAll('li .title')].map((e) => e.textContent);
const sent = () => fake.calls.filter((c) => c.cmd[0] !== 'status').map((c) => c.cmd);
const settle = async () => {
	for (let i = 0; i < 5; i++) {
		await tick();
		await new Promise((r) => setTimeout(r, 0));
	}
};

async function setup() {
	const r = render(QueuePage);
	await waitFor(() => expect(titles(r.container)).toEqual(['Song 1', 'Song 2', 'Song 3', 'Song 4']));
	fake.calls.length = 0;
	return r;
}

function pointer(el: Element, type: string, init: PointerEventInit) {
	el.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, ...init }));
	flushSync();
}

test('move buttons reorder at once and send playlist move', async () => {
	const r = await setup();
	await fireEvent.click(r.getByRole('button', { name: 'Move Song 1 down' }));
	expect(titles(r.container)).toEqual(['Song 2', 'Song 1', 'Song 3', 'Song 4']);
	await settle();
	expect(sent()).toEqual([['playlist', 'move', 0, 1]]);
	expect(r.getByRole('button', { name: 'Move Song 1 up' })).toBeTruthy();
});

test('a refused move is put back and reported', async () => {
	const r = await setup();
	refuse = ['playlist move'];
	await fireEvent.click(r.getByRole('button', { name: 'Move Song 2 up' }));
	expect(titles(r.container)[0]).toBe('Song 2');
	await waitFor(() => expect(titles(r.container)).toEqual(['Song 1', 'Song 2', 'Song 3', 'Song 4']));
	expect(r.getByRole('alert').textContent).toContain("Couldn't move the track");
});

test('Alt+ArrowDown on a focused row moves it, and focus follows', async () => {
	const r = await setup();
	const row = r.getByRole('button', { name: 'Play Song 2' });
	row.focus();
	await fireEvent.keyDown(row, { key: 'ArrowDown', altKey: true });
	await settle();
	expect(titles(r.container)).toEqual(['Song 1', 'Song 3', 'Song 2', 'Song 4']);
	expect(sent()).toEqual([['playlist', 'move', 1, 2]]);
	expect(document.activeElement?.getAttribute('aria-label')).toBe('Play Song 2');
	// Without Alt, arrows are left alone (scrolling, screen readers).
	await fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' });
	await settle();
	expect(sent()).toHaveLength(1);
});

test('dragging the handle shows the drop position and sends the move on release', async () => {
	const r = await setup();
	const handle = r.container.querySelectorAll('li .handle')[0]!;
	pointer(handle, 'pointerdown', { clientY: 25, pointerType: 'touch', button: 0 });
	pointer(handle, 'pointermove', { clientY: 140, pointerType: 'touch' }); // the row's middle is now at 140
	const rows = r.container.querySelectorAll('li');
	expect(rows[0]!.classList.contains('lifted')).toBe(true);
	expect(rows[2]!.classList.contains('drop-after'), 'marker below Song 3').toBe(true);
	expect(sent()).toEqual([]);
	pointer(handle, 'pointerup', { clientY: 140, pointerType: 'touch' });
	await settle();
	expect(titles(r.container)).toEqual(['Song 2', 'Song 3', 'Song 1', 'Song 4']);
	expect(sent()).toEqual([['playlist', 'move', 0, 2]]);
});

test('a drag back to where it started, or a cancelled one, sends nothing', async () => {
	const r = await setup();
	const handle = r.container.querySelectorAll('li .handle')[1]!;
	pointer(handle, 'pointerdown', { clientY: 75, button: 0 });
	pointer(handle, 'pointermove', { clientY: 200 });
	pointer(handle, 'pointermove', { clientY: 78 });
	pointer(handle, 'pointerup', { clientY: 78 });
	pointer(handle, 'pointerdown', { clientY: 75, button: 0 });
	pointer(handle, 'pointermove', { clientY: 200 });
	pointer(handle, 'pointercancel', {});
	await settle();
	expect(sent()).toEqual([]);
	expect(titles(r.container)).toEqual(['Song 1', 'Song 2', 'Song 3', 'Song 4']);
	// Another pointer can't hijack a drag in progress.
	pointer(handle, 'pointerdown', { clientY: 75, button: 0 });
	handle.dispatchEvent(new PointerEvent('pointerup', { bubbles: true, pointerId: 9, clientY: 200 }));
	pointer(handle, 'pointerup', { clientY: 75 });
	await settle();
	expect(sent()).toEqual([]);
});

test('status refreshes during a drag are held back, then applied', async () => {
	const r = await setup();
	const handle = r.container.querySelectorAll('li .handle')[0]!;
	pointer(handle, 'pointerdown', { clientY: 25, button: 0 });
	queue = [track(9), ...queue];
	fake.app.queueVersion++; // the 2-second refresh, or an LMS event
	await settle();
	expect(titles(r.container)).toEqual(['Song 1', 'Song 2', 'Song 3', 'Song 4']);
	pointer(handle, 'pointercancel', {});
	await waitFor(() => expect(titles(r.container)[0]).toBe('Song 9'));
});

test('swiping a row left past the threshold removes it, with Undo', async () => {
	const r = await setup();
	const main = r.getByRole('button', { name: 'Play Song 3' });
	pointer(main, 'pointerdown', { clientX: 300, clientY: 110, pointerType: 'touch' });
	pointer(main, 'pointermove', { clientX: 250, clientY: 112, pointerType: 'touch' });
	expect(main.closest('li')!.classList.contains('swiping')).toBe(true);
	pointer(main, 'pointermove', { clientX: 150, clientY: 112, pointerType: 'touch' });
	pointer(main, 'pointerup', { clientX: 150, clientY: 112, pointerType: 'touch' });
	expect(main.closest('li')!.classList.contains('leaving')).toBe(true);
	await waitFor(() => expect(titles(r.container)).toEqual(['Song 1', 'Song 2', 'Song 4']));
	await settle();
	expect(sent()).toEqual([['playlist', 'delete', 2]]);
	expect(r.getByRole('status').textContent).toContain('Removed Song 3');

	// Undo: add the track back (by id) and move it from the end to where it was.
	fake.calls.length = 0;
	await fireEvent.click(r.getByRole('button', { name: 'Undo' }));
	expect(titles(r.container)).toEqual(['Song 1', 'Song 2', 'Song 3', 'Song 4']);
	await settle();
	expect(sent()).toEqual([
		['playlistcontrol', 'cmd:add', 'track_id:3'],
		['playlist', 'move', 3, 2],
	]);
	expect(r.queryByRole('button', { name: 'Undo' })).toBeNull();
});

test('a short swipe, a scroll, or a mouse drag does not remove', async () => {
	const r = await setup();
	const main = r.getByRole('button', { name: 'Play Song 2' });
	// Short: snaps back.
	pointer(main, 'pointerdown', { clientX: 300, clientY: 60, pointerType: 'touch' });
	pointer(main, 'pointermove', { clientX: 240, clientY: 60, pointerType: 'touch' });
	pointer(main, 'pointerup', { clientX: 240, clientY: 60, pointerType: 'touch' });
	// Mostly vertical: a scroll, not ours.
	pointer(main, 'pointerdown', { clientX: 300, clientY: 60, pointerType: 'touch' });
	pointer(main, 'pointermove', { clientX: 200, clientY: 300, pointerType: 'touch' });
	pointer(main, 'pointerup', { clientX: 100, clientY: 400, pointerType: 'touch' });
	// Mouse: never a swipe.
	pointer(main, 'pointerdown', { clientX: 300, clientY: 60, pointerType: 'mouse', button: 0 });
	pointer(main, 'pointermove', { clientX: 0, clientY: 60, pointerType: 'mouse' });
	pointer(main, 'pointerup', { clientX: 0, clientY: 60, pointerType: 'mouse' });
	// Cancelled by the browser (it took over to scroll).
	pointer(main, 'pointerdown', { clientX: 300, clientY: 60, pointerType: 'touch' });
	pointer(main, 'pointermove', { clientX: 100, clientY: 60, pointerType: 'touch' });
	pointer(main, 'pointercancel', { pointerType: 'touch' });
	await new Promise((r) => setTimeout(r, 250));
	await settle();
	expect(sent().filter((c) => c[1] === 'delete')).toEqual([]);
	expect(titles(r.container)).toHaveLength(4);
});

test('the tap that ends a swipe does not also play the track', async () => {
	const r = await setup();
	const main = r.getByRole('button', { name: 'Play Song 2' });
	pointer(main, 'pointerdown', { clientX: 300, clientY: 60, pointerType: 'touch' });
	pointer(main, 'pointermove', { clientX: 260, clientY: 60, pointerType: 'touch' });
	pointer(main, 'pointerup', { clientX: 260, clientY: 60, pointerType: 'touch' });
	await fireEvent.click(main);
	await settle();
	expect(sent()).toEqual([]);
});

test('a refused removal is put back and reported, with no Undo', async () => {
	const r = await setup();
	refuse = ['playlist delete'];
	await fireEvent.click(r.getByRole('button', { name: 'Remove Song 1' }));
	expect(titles(r.container)).toEqual(['Song 2', 'Song 3', 'Song 4']);
	await waitFor(() => expect(titles(r.container)).toEqual(['Song 1', 'Song 2', 'Song 3', 'Song 4']));
	expect(r.getByRole('alert').textContent).toContain("Couldn't remove the track");
	expect(r.queryByRole('button', { name: 'Undo' })).toBeNull();
});

test('a failed Undo is put back and reported', async () => {
	const r = await setup();
	await fireEvent.click(r.getByRole('button', { name: 'Remove Song 2' }));
	await settle();
	refuse = ['playlistcontrol'];
	await fireEvent.click(r.getByRole('button', { name: 'Undo' }));
	await waitFor(() => expect(titles(r.container)).toEqual(['Song 1', 'Song 3', 'Song 4']));
	expect(r.getByRole('alert').textContent).toContain("Couldn't put the track back");
});

test('Undo waits for LMS to apply a late add before moving the track into place', async () => {
	const r = await setup();
	await fireEvent.click(r.getByRole('button', { name: 'Remove Song 1' }));
	await settle();
	const base = fake.handler.rpc;
	let late: Track | null = null;
	fake.handler.rpc = async (p, c) => {
		if (c[0] === 'playlistcontrol') {
			late = track(1); // applied on the next status query, not now
			return {};
		}
		if (c[0] === 'status' && late) {
			const res = await base(p, c);
			queue = [...queue, late];
			late = null;
			return res;
		}
		return base(p, c);
	};
	fake.calls.length = 0;
	await fireEvent.click(r.getByRole('button', { name: 'Undo' }));
	await waitFor(() => expect(sent()).toContainEqual(['playlist', 'move', 3, 0]), { timeout: 2000 });
	await waitFor(() => expect(titles(r.container)).toEqual(['Song 1', 'Song 2', 'Song 3', 'Song 4']));
	expect(r.queryByRole('alert')).toBeNull();
});

test('Undo never moves a track it cannot be sure is the one it added', async () => {
	const r = await setup();
	await fireEvent.click(r.getByRole('button', { name: 'Remove Song 1' }));
	await settle();
	const base = fake.handler.rpc;
	fake.handler.rpc = async (p, c) => {
		if (c[0] === 'playlistcontrol') queue = [...queue, track(8)]; // someone else added one too
		return base(p, c);
	};
	fake.calls.length = 0;
	await fireEvent.click(r.getByRole('button', { name: 'Undo' }));
	await waitFor(() => expect(r.getByRole('alert').textContent).toContain('added at the end'));
	expect(sent().filter((c) => c[1] === 'move')).toEqual([]);
	await waitFor(() => expect(titles(r.container)).toEqual(['Song 2', 'Song 3', 'Song 4', 'Song 8', 'Song 1']));
});

test('no Undo for tracks without a library id (e.g. radio streams)', async () => {
	queue = [track(1), track(2, -5)];
	const r = render(QueuePage);
	await waitFor(() => expect(titles(r.container)).toHaveLength(2));
	await fireEvent.click(r.getByRole('button', { name: 'Remove Song 2' }));
	await settle();
	expect(r.getByRole('status').textContent).toContain('Removed Song 2');
	expect(r.queryByRole('button', { name: 'Undo' })).toBeNull();
});

test('edits are sent one at a time, in order', async () => {
	const r = await setup();
	let release!: () => void;
	const base = fake.handler.rpc;
	fake.handler.rpc = async (p, c) => {
		if (c[1] === 'move' && c[2] === 0) await new Promise<void>((res) => (release = res));
		return base(p, c);
	};
	await fireEvent.click(r.getByRole('button', { name: 'Move Song 1 down' }));
	await fireEvent.click(r.getByRole('button', { name: 'Remove Song 4' }));
	await settle();
	expect(sent()).toEqual([['playlist', 'move', 0, 1]]);
	// A refresh while edits are in flight would show LMS's old order: it is held back.
	fake.app.queueVersion++;
	await settle();
	expect(titles(r.container)).toEqual(['Song 2', 'Song 1', 'Song 3']);
	release();
	await settle();
	expect(sent()).toEqual([
		['playlist', 'move', 0, 1],
		['playlist', 'delete', 3],
	]);
});

test('hostile track titles stay text in rows, labels and the snackbar', async () => {
	queue = [{ id: 1, title: '<img src=x onerror=alert(1)>' }];
	const r = render(QueuePage);
	await waitFor(() => expect(titles(r.container)).toHaveLength(1));
	await fireEvent.click(r.getByRole('button', { name: 'Remove <img src=x onerror=alert(1)>' }));
	await settle();
	expect(document.querySelectorAll('img').length).toBe(0);
	expect(r.getByRole('status').textContent).toContain('<img src=x');
});

// Regression: removal is by position, and the queue can change under the user (another app,
// a station topping it up). It removed whichever track had moved into that position.
test('a track no longer at its position is not removed (nor is anything else)', async () => {
	const r = await setup();
	// Another app puts a track at the front: Song 3 is now at index 3, not 2.
	queue = [track(9), ...queue];
	await fireEvent.click(r.getByRole('button', { name: 'Remove Song 3' }));
	await settle();
	expect(sent().filter((c) => c[1] === 'delete')).toEqual([]);
	expect(queue.map((t) => t.title)).toEqual(['Song 9', 'Song 1', 'Song 2', 'Song 3', 'Song 4']);
	expect(r.getByRole('alert').textContent).toContain('the queue changed meanwhile');
	// …and the page shows the queue as it is now.
	await waitFor(() => expect(titles(r.container)).toEqual(['Song 9', 'Song 1', 'Song 2', 'Song 3', 'Song 4']));
});

test('a move checks the track is still where it was', async () => {
	const r = await setup();
	queue = [track(9), ...queue];
	const row = r.getByRole('button', { name: 'Play Song 2' });
	row.focus();
	await fireEvent.keyDown(row, { key: 'ArrowDown', altKey: true });
	await settle();
	expect(sent().filter((c) => c[1] === 'move')).toEqual([]);
	await waitFor(() => expect(titles(r.container)[0]).toBe('Song 9'));
});
