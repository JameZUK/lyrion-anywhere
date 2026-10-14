// Every Snackbar on the page, in the order their messages arrived (Wave 3.5, rule 1: one
// message at a time). Each snackbar joins when it is made and leaves when it goes; `top` is
// the newest one with a message, the only one shown. When it is dismissed, the one before it
// (if it still has a message) shows again.
import { untrack } from 'svelte';

class Snackbars {
	#next = 1;
	/** Ids of the snackbars with a message, oldest first. */
	#order = $state<number[]>([]);

	join(): number {
		return this.#next++;
	}

	leave(id: number) {
		this.set(id, false);
	}

	/** Whether snackbar `id` has a message now (a new message puts it on top). */
	set(id: number, has: boolean) {
		// Called from the snackbars' effects: reading the list here mustn't make them depend on it.
		untrack(() => {
			const at = this.#order.indexOf(id);
			if (has && at === -1) this.#order = [...this.#order, id];
			else if (!has && at !== -1) this.#order = this.#order.filter((x) => x !== id);
		});
	}

	/** The snackbar to show, or 0 for none. */
	get top(): number {
		return this.#order.at(-1) ?? 0;
	}

	/** How many have a message (tests). */
	get count(): number {
		return this.#order.length;
	}
}

export const snackbars = new Snackbars();
