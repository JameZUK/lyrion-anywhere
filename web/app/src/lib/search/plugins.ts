// "Search on <plugin>" (Wave 2 §3.3, U7): hand the library search's text to the search box an
// app or radio service offers in its own top menu. Nothing is hard-coded per plugin: an app
// offers search when LMS lists a search box among its top menu's items, whose action opens a
// menu with the typed text as its `search:` parameter (and nowhere else). The text is sent
// only through that action, exactly as the menu offered it; the gateway runs it only because
// LMS offered it to this device (and checks the text as it checks any search text).
import type { Command } from '$lib/api/rpc';
import { href } from '$lib/nav';
import type { AppInfo } from '$lib/lms/types';
import { itemKind, resolveAction, toCommand, type Resolved, type SBMenu } from '$lib/lms/slimbrowse';

/** An app whose top menu has a search box. */
export interface SearchablePlugin {
	cmd: string;
	name: string;
	icon: unknown;
	/** Its search box's action (opens the results, with the text as `search:`). */
	search: Resolved;
}

const MARKER = /^(__TAGGEDINPUT__|__INPUT__)$/;
const ANY_MARKER = /__TAGGEDINPUT__|__INPUT__/;
const APP = /^[a-z0-9_]{1,64}$/;
/** Longest search text sent (as typed text elsewhere). */
export const MAX_QUERY = 200;
/** Apps looked at, at most (LMS lists a handful; anything more is not believed). */
export const MAX_APPS = 64;
/** Top menus read at once (LMS is single-threaded). */
export const CONCURRENCY = 3;

/**
 * Whether an action is a search box for `search:` text: it opens a menu, and its one and only
 * typed-text parameter is `search`. Anything else (text going into another parameter, or into
 * several) is not used for "Search on".
 */
export function isSearchBox(r: Resolved | null): r is Resolved {
	if (!r || !r.menu) return false;
	const typed = Object.entries(r.params).filter(([, v]) => ANY_MARKER.test(v));
	return typed.length === 1 && typed[0]![0] === 'search' && MARKER.test(typed[0]![1]);
}

/** The first search box among a top menu's items, or null if it has none. */
export function searchBox(menu: SBMenu | null | undefined): Resolved | null {
	const items = Array.isArray(menu?.item_loop) ? menu!.item_loop : [];
	for (const item of items) {
		if (!item || typeof item !== 'object' || itemKind(menu!, item) !== 'search') continue;
		const go = resolveAction(menu!, item, 'go');
		if (isSearchBox(go)) return go;
	}
	return null;
}

/** The search text as it is sent: control characters as spaces, trimmed, bounded. */
export function cleanQuery(q: unknown): string {
	if (typeof q !== 'string') return '';
	return q
		.replace(/[\u0000-\u001f\u007f-\u009f]/g, ' ')
		.trim()
		.slice(0, MAX_QUERY)
		.trim();
}

/**
 * The command searching with `box` for `query` (a page of results). Refuses anything but a
 * search box, and empty text: nothing else is ever sent.
 */
export function searchCommand(box: Resolved, query: string, page: { start: number; count: number }): Command {
	if (!isSearchBox(box)) throw new RangeError('not a search box');
	const q = cleanQuery(query);
	if (!q) throw new RangeError('enter something to search for');
	return toCommand(box, page, q);
}

const validApp = (a: AppInfo): a is AppInfo & { cmd: string } => typeof a?.cmd === 'string' && APP.test(a.cmd);

/** The apps worth asking, once each, in the order LMS lists them. */
export function candidates(lists: AppInfo[][]): (AppInfo & { cmd: string })[] {
	const seen = new Set<string>();
	const out: (AppInfo & { cmd: string })[] = [];
	for (const a of lists.flat()) {
		if (!validApp(a) || seen.has(a.cmd)) continue;
		seen.add(a.cmd);
		out.push(a);
		if (out.length >= MAX_APPS) break;
	}
	return out;
}

/** What identifies the apps listed: when it changes, they are looked at again. */
export const signature = (apps: { cmd: string }[]) => apps.map((a) => a.cmd).join(',');

/**
 * Read each app's top menu (`open`, a few at a time) and keep those with a search box, in
 * the order listed. An app whose menu can't be read is left out.
 */
export async function discover(apps: (AppInfo & { cmd: string })[], open: (cmd: string) => Promise<SBMenu>, concurrency = CONCURRENCY): Promise<SearchablePlugin[]> {
	const found: (SearchablePlugin | null)[] = apps.map(() => null);
	let next = 0;
	const worker = async () => {
		for (let i = next++; i < apps.length; i = next++) {
			const a = apps[i]!;
			try {
				const box = searchBox(await open(a.cmd));
				if (box) found[i] = { cmd: a.cmd, name: typeof a.name === 'string' && a.name.trim() ? a.name.trim() : a.cmd, icon: a.icon, search: box };
			} catch {
				/* not usable from here: left out */
			}
		}
	};
	await Promise.all(Array.from({ length: Math.max(1, concurrency) }, worker));
	return found.filter((p): p is SearchablePlugin => p !== null);
}

/** How the directory reaches LMS (for tests: a fake). */
export interface DirectorySource {
	/** The `apps` and `radios` lists. */
	lists(): Promise<AppInfo[][]>;
	/** An app's top menu, for a player. */
	open(player: string, cmd: string): Promise<SBMenu>;
}

/**
 * Which apps can be searched, found once per session: looked at again only when the apps
 * listed change (installed, removed, or hidden from this account by the gateway), or after
 * `reset` (another user signed in).
 */
export class SearchDirectory {
	#sig: string | null = null;
	#list: SearchablePlugin[] = [];
	#pending: Promise<SearchablePlugin[]> | null = null;
	#generation = 0;

	constructor(private source: DirectorySource) {}

	/** The searchable apps (the apps lists are read each time; their menus only when changed). */
	async get(player: string): Promise<SearchablePlugin[]> {
		if (!player) return [];
		if (this.#pending) return this.#pending;
		const gen = this.#generation;
		const run = (async () => {
			const apps = candidates(await this.source.lists());
			const sig = signature(apps);
			if (sig === this.#sig) return this.#list;
			const list = await discover(apps, (cmd) => this.source.open(player, cmd));
			if (gen === this.#generation) {
				this.#sig = sig;
				this.#list = list;
			}
			return list;
		})();
		this.#pending = run;
		try {
			return await run;
		} finally {
			if (this.#pending === run) this.#pending = null;
		}
	}

	/**
	 * One app's search box, read again from its top menu now (so it is what LMS offers at
	 * this moment, and the gateway has just seen it offered), with the app's name.
	 */
	async find(player: string, cmd: string): Promise<SearchablePlugin | null> {
		if (!player || !APP.test(cmd)) return null;
		// Its name (and whether it is listed for this account at all) from the directory.
		const known = this.#list.find((p) => p.cmd === cmd) ?? (await this.get(player)).find((p) => p.cmd === cmd);
		if (!known) return null;
		const box = searchBox(await this.source.open(player, cmd));
		if (!box) return null;
		return { ...known, search: box };
	}

	/** Forget everything (another user signed in). */
	reset() {
		this.#generation++;
		this.#sig = null;
		this.#list = [];
		this.#pending = null;
	}
}

/** The in-app link to an app's results for `query` (opened by the `search-on` page). */
export function searchOnHref(app: string, query: string): string {
	return href(`/search-on/${encodeURIComponent(app)}/${encodeURIComponent(cleanQuery(query))}`);
}
