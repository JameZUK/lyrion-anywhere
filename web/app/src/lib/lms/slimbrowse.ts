// LMS menus ("SlimBrowse"): apps, radio, plugin menus and context menus.
//
// Each item offers actions (go, play, add, add-hold, more, …). An action is either on the item
// itself, or on the menu's `base` combined with the item's parameters (the field named by
// `itemsParams`). The gateway applies the same rules and only lets a device run actions LMS
// offered it, so this module never invents commands: it only replays what a menu offered.
import type { Command } from '$lib/api/rpc';

export interface SBAction {
	cmd?: unknown;
	params?: Record<string, unknown>;
	itemsParams?: string;
	nextWindow?: string;
	player?: number;
}

export interface SBItem {
	text?: string;
	type?: string;
	style?: string;
	icon?: string;
	'icon-id'?: string | number;
	image?: string;
	actions?: Record<string, SBAction | unknown>;
	input?: { title?: string; len?: number } | unknown;
	addAction?: string;
	nextWindow?: string;
	[k: string]: unknown;
}

export interface SBMenu {
	count?: number;
	offset?: number;
	title?: string;
	base?: { actions?: Record<string, SBAction | unknown> };
	item_loop?: SBItem[];
	window?: { windowStyle?: string; textarea?: string } | unknown;
}

/** A runnable action: command words and parameters, with where to go afterwards. */
export interface Resolved {
	cmd: string[];
	params: Record<string, string>;
	nextWindow?: string;
	/** Opens a menu (paged) rather than running a command. */
	menu: boolean;
}

const WORD = /^[A-Za-z0-9_-]{1,64}$/;
const KEY = /^[A-Za-z0-9_-]{1,64}$/;
const INPUT = /__TAGGEDINPUT__|__INPUT__/;

function scalar(v: unknown): string | undefined {
	if (typeof v === 'string') return v;
	if (typeof v === 'number' && Number.isFinite(v)) return String(v);
	if (typeof v === 'boolean') return v ? '1' : '0';
	return undefined;
}

function toResolved(name: string, item: SBItem, a: unknown, extra?: unknown): Resolved | null {
	if (!a || typeof a !== 'object') return null;
	const act = a as SBAction;
	if (!Array.isArray(act.cmd) || !act.cmd.length || act.cmd.length > 6) return null;
	if (!act.cmd.every((w) => typeof w === 'string' && WORD.test(w))) return null;
	const params: Record<string, string> = {};
	for (const src of [act.params, extra]) {
		if (!src || typeof src !== 'object') continue;
		for (const [k, v] of Object.entries(src as Record<string, unknown>)) {
			const s = scalar(v);
			if (s === undefined) continue;
			if (!KEY.test(k) || k === 'url') return null;
			params[k] = s;
		}
	}
	const cmd = act.cmd as string[];
	const next = typeof act.nextWindow === 'string' ? act.nextWindow : typeof item.nextWindow === 'string' ? item.nextWindow : undefined;
	// As in LMS's own clients (and the gateway): `… items` lists are menus, and so is any `go`
	// that doesn't say where to go next; with `nextWindow` it is a command that runs.
	const menu = cmd[cmd.length - 1] === 'items' || (name === 'go' && next === undefined);
	return { cmd, params, nextWindow: next, menu };
}

/** The action `name` for an item in a menu, or null if the item doesn't offer it. */
export function resolveAction(menu: SBMenu, item: SBItem, name: string): Resolved | null {
	const own = item.actions && typeof item.actions === 'object' ? (item.actions as Record<string, unknown>)[name] : undefined;
	if (own !== undefined) return toResolved(name, item, own);
	const base = menu.base?.actions?.[name];
	if (!base || typeof base !== 'object') return null;
	const field = typeof (base as SBAction).itemsParams === 'string' ? (base as SBAction).itemsParams! : 'params';
	const itemParams = item[field];
	if (!itemParams || typeof itemParams !== 'object') return null;
	return toResolved(name, item, base, itemParams);
}

/** Does running this action open another menu (rather than do something)? */
export const opensMenu = (r: Resolved) => r.menu;

/** Does it need text typed by the user (a search box)? */
export const needsInput = (r: Resolved) => Object.values(r.params).some((v) => INPUT.test(v));

/** The command to send: words, paging for menus, then parameters (search text filled in). */
export function toCommand(r: Resolved, page?: { start: number; count: number }, input?: string): Command {
	const out: Command = [...r.cmd];
	if (opensMenu(r)) out.push(Math.max(0, page?.start ?? 0), Math.max(1, Math.min(500, page?.count ?? 100)));
	for (const [k, v] of Object.entries(r.params)) {
		out.push(`${k}:${INPUT.test(v) ? (input ?? '').replace(/[\u0000-\u001f\u007f]/g, ' ').trim().slice(0, 200) : v}`);
	}
	return out;
}

export type ItemKind = 'folder' | 'search' | 'playable' | 'action' | 'text';

/** How to present an item. */
export function itemKind(menu: SBMenu, item: SBItem): ItemKind {
	const go = resolveAction(menu, item, 'go');
	if (go && needsInput(go)) return 'search';
	if (item.input && typeof item.input === 'object' && go) return 'search';
	// Audio items play when chosen, even if they can also be opened.
	if (item.type === 'audio' && resolveAction(menu, item, 'play')) return 'playable';
	if (go && opensMenu(go)) return 'folder';
	if (resolveAction(menu, item, 'play') || item.type === 'audio' || item.type === 'playlist') {
		return resolveAction(menu, item, 'play') || go ? 'playable' : 'text';
	}
	if (go) return 'action';
	return 'text';
}

/** Icon path (for the art relay) from an item's icon fields. */
export function iconOf(item: SBItem): unknown {
	return item.image ?? item.icon ?? item['icon-id'];
}

/** An entry plugins add to LMS's home menu (My Music, Radio, Extras, …). */
export interface PluginEntry {
	id: string;
	text: string;
	icon: unknown;
	node: string;
	weight: number;
	/** Opens this (a plugin menu), or null for a group of other entries. */
	go: Resolved | null;
	/** Entries under this one (e.g. Random Mix's mixes). */
	children: SBItem[];
}

const ID = /^[A-Za-z0-9_.-]{1,96}$/;

/**
 * Plugin entries from the home menu (`menu`), for listing beside apps and radio: everything
 * the gateway relayed, except what the app already shows its own way (the library, apps and
 * radio listed by `apps`/`radios`) and entries with nothing to open.
 */
export function pluginEntries(home: SBMenu, shownApps: Set<string>): PluginEntry[] {
	const loop = Array.isArray(home.item_loop) ? home.item_loop : [];
	const items = loop.filter((i) => !!i && typeof i === 'object' && typeof i.id === 'string' && ID.test(i.id));
	const out: PluginEntry[] = [];
	for (const item of items) {
		const id = item.id as string;
		const node = typeof item.node === 'string' ? item.node : '';
		if (id.startsWith('myMusic') || ['opmlmyapps', 'favorites', 'globalSearch', 'radios'].includes(id)) continue;
		// Entries under another entry are shown inside it.
		if (items.some((p) => p.id === node)) continue;
		const go = resolveAction({}, item, 'go');
		if (go && shownApps.has(go.cmd[0]!)) continue;
		const children = items.filter((c) => c.node === id);
		if (!go?.menu && !children.length) continue;
		out.push({
			id,
			text: typeof item.text === 'string' ? item.text : id,
			icon: item.icon ?? (item.window as { 'icon-id'?: unknown } | undefined)?.['icon-id'] ?? item.menuIcon,
			node,
			weight: typeof item.weight === 'number' ? item.weight : 100,
			go: go?.menu ? go : null,
			children,
		});
	}
	return out.sort((a, b) => a.weight - b.weight || a.text.localeCompare(b.text));
}
