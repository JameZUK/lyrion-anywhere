// The player switcher is a dialog: focus moves into it, Escape closes it, and focus goes
// back to the button that opened it.
import { render } from '@testing-library/svelte';
import { tick } from 'svelte';
import PlayerSwitcher from '$lib/components/PlayerSwitcher.svelte';

test('focus moves in on open, Escape closes, and focus returns to the opener', async () => {
	const opener = document.createElement('button');
	opener.textContent = 'Player';
	document.body.append(opener);
	opener.focus();
	const onclose = vi.fn();
	const r = render(PlayerSwitcher, { onclose });
	await tick();
	const dialog = r.getByRole('dialog', { name: 'Choose a player' });
	expect(dialog.contains(document.activeElement)).toBe(true);

	window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
	expect(onclose).toHaveBeenCalledTimes(1);
	window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
	expect(onclose).toHaveBeenCalledTimes(1);

	r.unmount(); // the parent closes it
	await tick();
	expect(document.activeElement).toBe(opener);
	opener.remove();
});

// Regression: the panel stayed open when you tapped outside it.
test('a tap outside closes it; taps inside, or on the button that opened it, do not', async () => {
	const opener = document.createElement('button');
	opener.setAttribute('aria-expanded', 'true');
	const elsewhere = document.createElement('div');
	document.body.append(opener, elsewhere);
	const onclose = vi.fn();
	const r = render(PlayerSwitcher, { onclose });
	await tick();
	const dialog = r.getByRole('dialog', { name: 'Choose a player' });
	const tap = (el: Element) => el.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
	tap(dialog.querySelector('h2')!);
	expect(onclose).not.toHaveBeenCalled();
	tap(opener);
	expect(onclose, 'the opener toggles it itself').not.toHaveBeenCalled();
	tap(elsewhere);
	expect(onclose).toHaveBeenCalledTimes(1);
	r.unmount();
	opener.remove();
	elsewhere.remove();
});
