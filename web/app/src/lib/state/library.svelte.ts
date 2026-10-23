// Which LMS virtual library to browse ("Libraries" in LMS's My Music). Shown only when LMS
// reports any; the choice is remembered per user on this device. Browser storage may be
// missing or refuse access (private windows, blocked site data): then nothing is remembered.
import * as cmd from '#lib/lms/commands';
import type { LibrariesResult } from '#lib/lms/types';

export interface Library {
	id: string;
	name: string;
}

type Store = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

function storage(): Store | undefined {
	try {
		return globalThis.localStorage;
	} catch {
		return undefined;
	}
}

const key = (user: string) => `anywhere.library.${encodeURIComponent(user.slice(0, 200))}`;

/** The remembered library for a user, if it is still a well-formed id. */
export function loadChoice(user: string, store: Store | undefined = storage()): string | null {
	if (!user || !store) return null;
	try {
		const v = store.getItem(key(user));
		return v === null ? null : cmd.libraryId(v);
	} catch {
		return null;
	}
}

export function saveChoice(user: string, id: string | null, store: Store | undefined = storage()) {
	if (!user || !store) return;
	try {
		if (id === null) store.removeItem(key(user));
		else store.setItem(key(user), cmd.libraryId(id));
	} catch {
		// Not remembered; the choice still applies until the page is reloaded.
	}
}

/** LMS's answer, cut down to libraries with well-formed ids, names shown as plain text. */
export function parseLibraries(r: LibrariesResult | null | undefined): Library[] {
	const out: Library[] = [];
	for (const x of r?.folder_loop ?? []) {
		try {
			const id = cmd.libraryId(x?.id);
			const name = typeof x?.name === 'string' && x.name.trim() ? x.name.trim().slice(0, 200) : 'Library';
			if (!out.some((l) => l.id === id)) out.push({ id, name });
		} catch {
			// skipped
		}
	}
	return out;
}

class LibraryChoice {
	/** Libraries LMS reports; empty when it has none (the picker is then hidden). */
	available = $state<Library[]>([]);
	#chosen = $state<string | null>(null);
	#user = '';

	/** The library browse commands are narrowed to, or null for the whole library. */
	get id(): string | null {
		const c = this.#chosen;
		return c && this.available.some((l) => l.id === c) ? c : null;
	}

	get name(): string | null {
		return this.available.find((l) => l.id === this.id)?.name ?? null;
	}

	/** Load the user's choice, then ask LMS which libraries exist. */
	async start(user: string, rpc: (c: (string | number)[]) => Promise<unknown>) {
		this.#user = user;
		this.#chosen = loadChoice(user);
		try {
			this.available = parseLibraries((await rpc(cmd.libraries())) as LibrariesResult);
		} catch {
			this.available = [];
		}
	}

	choose(id: string | null) {
		const valid = id === null || this.available.some((l) => l.id === id) ? id : null;
		this.#chosen = valid;
		saveChoice(this.#user, valid);
	}

	stop() {
		this.#user = '';
		this.#chosen = null;
		this.available = [];
	}
}

export const library = new LibraryChoice();
