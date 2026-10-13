// Recent searches (Wave 2 §3.3): kept on this device only, never sent to the gateway. At most
// 20, newest first, each removable, with "Clear all". Kept per signed-in user, so someone else
// signing in on the same browser doesn't see them.

export const MAX_RECENTS = 20;
/** Longest term kept. */
export const MAX_TERM = 200;
const PREFIX = 'anywhere.search.recents';

function local(): Storage | undefined {
	try {
		return globalThis.localStorage;
	} catch {
		return undefined;
	}
}

/** A term as kept: control characters out, spaces collapsed, trimmed and length-capped. */
export function cleanTerm(v: unknown): string {
	if (typeof v !== 'string') return '';
	return v.replace(/[\u0000-\u001f\u007f-\u009f]/g, ' ').replace(/\s+/g, ' ').trim().slice(0, MAX_TERM);
}

const same = (a: string, b: string) => a.toLocaleLowerCase('en-GB') === b.toLocaleLowerCase('en-GB');

/** `term` added at the front (an earlier copy, in any case, moved rather than repeated). */
export function addRecent(list: readonly string[], term: unknown): string[] {
	const t = cleanTerm(term);
	if (!t) return [...list];
	return [t, ...list.filter((x) => !same(x, t))].slice(0, MAX_RECENTS);
}

export function removeRecent(list: readonly string[], term: string): string[] {
	return list.filter((x) => !same(x, term));
}

/** Whatever storage holds, as a clean list. */
export function parseRecents(raw: unknown): string[] {
	if (!Array.isArray(raw)) return [];
	let out: string[] = [];
	for (const v of raw.slice(0, 100)) {
		const t = cleanTerm(v);
		if (t && !out.some((x) => same(x, t))) out.push(t);
	}
	out = out.slice(0, MAX_RECENTS);
	return out;
}

const key = (user: string) => `${PREFIX}.${encodeURIComponent(user.slice(0, 100))}`;

export function loadRecents(user: string, storage: Pick<Storage, 'getItem'> | undefined = local()): string[] {
	if (!user) return [];
	try {
		return parseRecents(JSON.parse(storage?.getItem(key(user)) ?? 'null'));
	} catch {
		return [];
	}
}

export function saveRecents(user: string, list: readonly string[], storage: Pick<Storage, 'setItem' | 'removeItem'> | undefined = local()) {
	if (!user) return;
	try {
		if (list.length) storage?.setItem(key(user), JSON.stringify(list.slice(0, MAX_RECENTS)));
		else storage?.removeItem(key(user));
	} catch {
		/* storage blocked: recents last until the page closes */
	}
}
