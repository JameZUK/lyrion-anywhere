// A track's "More options" is a proper menu button: it works from the keyboard, says whether
// it is open, and gives focus back when it closes.
import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import TrackList from '$lib/components/TrackList.svelte';

const tracks = [
	{ id: 1, title: 'First' },
	{ id: 2, title: 'Second' },
];

async function setup() {
	const onnext = vi.fn();
	const onadd = vi.fn();
	const onmore = vi.fn();
	const r = render(TrackList, { tracks, onplay: () => {}, onnext, onadd, onmore });
	const button = r.getByRole('button', { name: 'More options for Second' });
	return { r, button, onnext, onadd, onmore };
}
const settle = async () => {
	await tick();
	await new Promise((r) => queueMicrotask(() => r(null)));
	await tick();
};

test('the button says it opens a menu, and whether it is open', async () => {
	const { r, button } = await setup();
	expect(button.getAttribute('aria-haspopup')).toBe('menu');
	expect(button.getAttribute('aria-expanded')).toBe('false');
	await fireEvent.click(button);
	await settle();
	expect(button.getAttribute('aria-expanded')).toBe('true');
	const menu = r.getByRole('menu');
	expect(button.getAttribute('aria-controls')).toBe(menu.id);
	expect(menu.getAttribute('aria-labelledby')).toBe(button.id);
	expect(document.activeElement?.textContent).toBe('Play next');
});

test('arrow keys open it and move through it; Home and End; Escape closes it and returns focus', async () => {
	const { r, button } = await setup();
	button.focus();
	await fireEvent.keyDown(button, { key: 'ArrowUp' });
	await settle();
	const menu = r.getByRole('menu');
	expect(document.activeElement?.textContent).toBe('More…');
	await fireEvent.keyDown(menu, { key: 'ArrowDown' });
	expect(document.activeElement?.textContent).toBe('Play next'); // wraps round
	await fireEvent.keyDown(menu, { key: 'ArrowDown' });
	expect(document.activeElement?.textContent).toBe('Add to queue');
	await fireEvent.keyDown(menu, { key: 'End' });
	expect(document.activeElement?.textContent).toBe('More…');
	await fireEvent.keyDown(menu, { key: 'Home' });
	expect(document.activeElement?.textContent).toBe('Play next');
	await fireEvent.keyDown(menu, { key: 'ArrowUp' });
	expect(document.activeElement?.textContent).toBe('More…');
	await fireEvent.keyDown(menu, { key: 'Escape' });
	await tick();
	expect(r.queryByRole('menu')).toBeNull();
	expect(document.activeElement).toBe(button);
	expect(button.getAttribute('aria-expanded')).toBe('false');
});

test('choosing an item runs it for that track, closes the menu and keeps focus on the button', async () => {
	const { r, button, onadd } = await setup();
	await fireEvent.click(button);
	await settle();
	await fireEvent.click(r.getByRole('menuitem', { name: 'Add to queue' }));
	await tick();
	expect(onadd).toHaveBeenCalledWith(tracks[1]);
	expect(r.queryByRole('menu')).toBeNull();
	expect(document.activeElement).toBe(button);
});

test('a click elsewhere or Tab closes it', async () => {
	const { r, button } = await setup();
	await fireEvent.click(button);
	await settle();
	await fireEvent.pointerDown(document.body);
	await tick();
	expect(r.queryByRole('menu')).toBeNull();
	await fireEvent.click(button);
	await settle();
	await fireEvent.keyDown(r.getByRole('menu'), { key: 'Tab' });
	await tick();
	expect(r.queryByRole('menu')).toBeNull();
});
