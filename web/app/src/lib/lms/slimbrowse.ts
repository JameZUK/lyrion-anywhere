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

function toResolved(a: unknown, extra?: unknown): Resolved | null {
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
	return { cmd: act.cmd as string[], params, nextWindow: typeof act.nextWindow === 'string' ? act.nextWindow : undefined };
}

/** The action `name` for an item in a menu, or null if the item doesn't offer it. */
export function resolveAction(menu: SBMenu, item: SBItem, name: string): Resolved | null {
	const own = item.actions && typeof item.actions === 'object' ? (item.actions as Record<string, unknown>)[name] : undefined;
	if (own !== undefined) return toResolved(own);
	const base = menu.base?.actions?.[name];
	if (!base || typeof base !== 'object') return null;
	const field = typeof (base as SBAction).itemsParams === 'string' ? (base as SBAction).itemsParams! : 'params';
	const itemParams = item[field];
	if (!itemParams || typeof itemParams !== 'object') return null;
	return toResolved(base, itemParams);
}

/** Does running this action open another menu (rather than do something)? */
export const opensMenu = (r: Resolved) => r.cmd[r.cmd.length - 1] === 'items';

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
