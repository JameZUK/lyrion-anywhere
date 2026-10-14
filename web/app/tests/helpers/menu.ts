// Actions that moved into a ⋯ menu (Wave 3.5): open the menu by its button's name, then choose
// an item by its name. Both are looked up in `scope` (a container, or the whole document).
import { fireEvent, screen, within } from '@testing-library/svelte';

type Name = string | RegExp;

/** The ⋯ button named `trigger`, and the menu it opened. */
export async function openMenu(trigger: Name, scope?: HTMLElement): Promise<HTMLElement> {
	const q = scope ? within(scope) : screen;
	const btn = q.getByRole('button', { name: trigger });
	await fireEvent.click(btn);
	const id = btn.getAttribute('aria-controls');
	const menu = id ? document.getElementById(id) : null;
	if (!menu) throw new Error(`no menu opened by ${String(trigger)}`);
	return menu;
}

/** Open the menu named `trigger` and choose `item` in it. */
export async function choose(trigger: Name, item: Name, scope?: HTMLElement): Promise<void> {
	const menu = await openMenu(trigger, scope);
	await fireEvent.click(within(menu).getByRole('menuitem', { name: item }));
}

/** The names of the items in the menu named `trigger` (it is left open). */
export async function menuItems(trigger: Name, scope?: HTMLElement): Promise<string[]> {
	const menu = await openMenu(trigger, scope);
	return within(menu)
		.getAllByRole('menuitem')
		.map((m) => (m.querySelector('span')?.textContent ?? m.textContent ?? '').trim());
}
