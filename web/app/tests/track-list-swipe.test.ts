// Browse track lists, Squeezer style: swipe right to play next, left to add to the queue, with
// N and A on a focused row (and the existing menu) as alternatives.
import { fireEvent, render } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import TrackList from '#lib/components/TrackList.svelte';
import type { Track } from '#lib/lms/types';

const tracks: Track[] = [
	{ id: 1, title: 'One', duration: 100 },
	{ id: 2, title: 'Two', duration: 100 },
];

function pointer(el: Element, type: string, init: PointerEventInit) {
	el.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, ...init }));
	flushSync();
}

function setup() {
	const done: string[] = [];
	const r = render(TrackList, {
		tracks,
		onplay: (t: Track) => done.push(`play ${t.id}`),
		onnext: (t: Track) => done.push(`next ${t.id}`),
		onadd: (t: Track) => done.push(`add ${t.id}`),
	});
	const row = r.container.querySelectorAll<HTMLElement>('li.row')[1]!;
	row.getBoundingClientRect = () => ({ top: 0, height: 50, left: 0, width: 300, right: 300, bottom: 50, x: 0, y: 0, toJSON() {} }) as DOMRect;
	return { r, done, row };
}

test('swipe right plays next, swipe left adds; the tap ending a swipe doesn’t play', async () => {
	const { done, row } = setup();
	const main = row.querySelector('.main')!;
	pointer(main, 'pointerdown', { clientX: 100, clientY: 10, pointerType: 'touch' });
	pointer(main, 'pointermove', { clientX: 220, clientY: 12, pointerType: 'touch' });
	pointer(main, 'pointerup', { clientX: 220, clientY: 12, pointerType: 'touch' });
	await fireEvent.click(main);
	expect(done).toEqual(['next 2']);
	pointer(main, 'pointerdown', { clientX: 250, clientY: 10, pointerType: 'pen' });
	pointer(main, 'pointermove', { clientX: 100, clientY: 12, pointerType: 'pen' });
	pointer(main, 'pointerup', { clientX: 100, clientY: 12, pointerType: 'pen' });
	expect(done).toEqual(['next 2', 'add 2']);
});

test('keys on a focused row: N plays next, A adds; the row says so', async () => {
	const { r, done, row } = setup();
	const main = row.querySelector<HTMLButtonElement>('.main')!;
	expect(main.getAttribute('aria-keyshortcuts')).toBe('N A');
	await fireEvent.keyDown(main, { key: 'n' });
	await fireEvent.keyDown(main, { key: 'a' });
	await fireEvent.keyDown(main, { key: 'a', metaKey: true });
	expect(done).toEqual(['next 2', 'add 2']);
	expect(r.container.textContent).toContain('press N to play it next');
});

test('without play next or add, rows have no swipes or keys', async () => {
	const done: string[] = [];
	const r = render(TrackList, { tracks, onplay: (t: Track) => done.push(`play ${t.id}`) });
	const main = r.container.querySelector<HTMLButtonElement>('.main')!;
	expect(main.hasAttribute('aria-keyshortcuts')).toBe(false);
	await fireEvent.keyDown(main, { key: 'n' });
	pointer(main, 'pointerdown', { clientX: 250, clientY: 10, pointerType: 'touch' });
	pointer(main, 'pointermove', { clientX: 50, clientY: 12, pointerType: 'touch' });
	pointer(main, 'pointerup', { clientX: 50, clientY: 12, pointerType: 'touch' });
	expect(done).toEqual([]);
});
