// The signed-in user's own data on the gateway (docs/PROTOCOL.md › Listening history, Home,
// Stations): listening history, the Home layout, stations and pinned menus. Nothing here is
// shared with other users.
import { ApiError, bearer, callJson } from './http';
import type { Command } from './rpc';
import { FILTER_LIMITS, cleanSpec, filterFrom, filterProblem, validId, type FilterSpec, type SmartFilter } from './filters';

export type { FilterRule, FilterSort, FilterSpec, SmartFilter } from './filters';

/** Runs `fn` with a valid access token (see SessionManager.authed). */
export type Authed = <T>(fn: (token: string) => Promise<T>) => Promise<T>;

export interface HistoryEntry {
	track_key: string;
	track_id: number;
	album_id: number | null;
	artist_id: number | null;
	year: number | null;
	/** When the play started (Unix seconds). */
	played_at: number;
	seconds: number;
	source: 'live' | 'offline';
}

export interface HistoryPage {
	enabled: boolean;
	retention_days: number;
	items: HistoryEntry[];
	/** Cursor for the next (older) page, or null at the end. */
	next: string | null;
}

export type ShelfName = 'recent' | 'most' | 'on_this_day';

export const SHELF_TYPES = ['recent', 'stations', 'new', 'most', 'on_this_day', 'random', 'favourites', 'playlists', 'station', 'menu', 'genre', 'filter'] as const;
export type ShelfType = (typeof SHELF_TYPES)[number];

export interface HomeShelf {
	type: ShelfType;
	id?: number;
	hidden?: boolean;
}

export interface HomeLayout {
	version: 1;
	shelves: HomeShelf[];
	/** True when the user has never saved a layout. */
	default?: boolean;
}

export type StationSpec =
	| { kind: 'genre'; genres: number[] }
	| { kind: 'decade'; decade: number; mine?: boolean }
	| { kind: 'forgotten' }
	| { kind: 'fresh' }
	| { kind: 'deep_cuts' }
	| { kind: 'menu'; steps: Command[]; play: Command; add?: Command | null };

export type StationKind = StationSpec['kind'];

export interface Station {
	id: number;
	name: string;
	kind: StationKind;
	spec: StationSpec;
	/** Why it can't play any more (a pinned menu item no longer offered), or null. */
	broken: string | null;
}

export interface PinnedMenu {
	id: number;
	name: string;
	steps: Command[];
	broken: string | null;
}

/** What to do with a station's next batch (see lib/stations/refill.ts). */
export type NextBatch =
	| { mode: 'tracks'; tracks: number[]; refill: true }
	| { mode: 'randomplay'; command: Command; refill: false }
	| { mode: 'ran'; refill: boolean }
	| { mode: 'none'; refill: false };

export interface NextRequest {
	player: string;
	start?: boolean;
	count?: number;
	exclude?: number[];
}

/** A page of a filter's albums: ids only (details are loaded by id, as for shelves). */
export interface FilterAlbums {
	albums: number[];
	count: number;
}

const PLAYER = /^[0-9a-f]{2}(:[0-9a-f]{2}){5}$/;
const CURSOR = /^\d{1,15}-\d{1,15}$/;

/** A station or pinned-menu id for a URL path: a positive integer, nothing else. */
export function pinId(v: unknown): number {
	if (typeof v !== 'number' || !Number.isSafeInteger(v) || v <= 0) throw new RangeError('bad id');
	return v;
}

function player(p: string): string {
	const v = p.toLowerCase();
	if (!PLAYER.test(v)) throw new RangeError('bad player id');
	return v;
}

/** A checked queue edit (docs/PROTOCOL.md "Checked queue edits"). */
export type QueueEditBody = { op: 'remove'; index: number; track: number } | { op: 'move'; index: number; to: number; track: number };

export class MeApi {
	constructor(private authed: Authed) {}

	/** This gateway predates checked queue edits (it answered 404 or 405). */
	#noQueueEdit = false;

	/**
	 * Remove or move a queue entry only if it is still `track`, checked and changed in one
	 * step on the gateway: 'done', 'changed' (nothing was changed), or 'unsupported' (an older
	 * gateway: the caller checks itself).
	 */
	async queueEdit(p: string, edit: QueueEditBody): Promise<'done' | 'changed' | 'unsupported'> {
		const path = `/v1/players/${encodeURIComponent(player(p))}/queue/edit`;
		if (this.#noQueueEdit) return 'unsupported';
		try {
			await this.#call(path, 'POST', edit);
			return 'done';
		} catch (e) {
			if (e instanceof ApiError && e.status === 409) return 'changed';
			if (e instanceof ApiError && (e.status === 404 || e.status === 405)) {
				this.#noQueueEdit = true;
				return 'unsupported';
			}
			// Not an answer from the gateway (no network, say): the caller's own check runs
			// and reports what happens.
			if (!(e instanceof ApiError)) return 'unsupported';
			throw e;
		}
	}

	#call<T>(path: string, method = 'GET', body?: unknown): Promise<T> {
		return this.authed((token) =>
			callJson<T>(path, {
				method,
				headers: { ...bearer(token), ...(body === undefined ? {} : { 'Content-Type': 'application/json' }) },
				...(body === undefined ? {} : { body: JSON.stringify(body) }),
			}),
		);
	}

	// --- history -----------------------------------------------------------------------

	history(limit = 50, before?: string | null): Promise<HistoryPage> {
		const n = Math.max(1, Math.min(200, Math.floor(limit)));
		const q = new URLSearchParams({ limit: String(n) });
		if (before) {
			if (!CURSOR.test(before)) throw new RangeError('bad cursor');
			q.set('before', before);
		}
		return this.#call(`/v1/me/history?${q}`);
	}

	clearHistory(): Promise<{ removed: number }> {
		return this.#call('/v1/me/history', 'DELETE');
	}

	setHistory(enabled: boolean): Promise<{ enabled: boolean }> {
		return this.#call('/v1/me/history', 'PATCH', { enabled: !!enabled });
	}

	/** Album ids for a shelf, best first. */
	async shelf(name: ShelfName, limit = 20): Promise<number[]> {
		if (!['recent', 'most', 'on_this_day'].includes(name)) throw new RangeError('bad shelf');
		const n = Math.max(1, Math.min(50, Math.floor(limit)));
		const r = await this.#call<{ albums?: unknown[] }>(`/v1/me/shelves/${name}?limit=${n}`);
		return (r.albums ?? []).filter((x): x is number => typeof x === 'number' && Number.isSafeInteger(x) && x > 0);
	}

	// --- Home --------------------------------------------------------------------------

	home(): Promise<HomeLayout> {
		return this.#call('/v1/me/home');
	}

	saveHome(layout: HomeLayout): Promise<HomeLayout> {
		return this.#call('/v1/me/home', 'PUT', { version: 1, shelves: layout.shelves });
	}

	resetHome(): Promise<HomeLayout> {
		return this.#call('/v1/me/home', 'DELETE');
	}

	// --- stations ------------------------------------------------------------------------

	async stations(): Promise<Station[]> {
		return (await this.#call<{ stations?: Station[] }>('/v1/me/stations')).stations ?? [];
	}

	createStation(name: string, spec: StationSpec, playerId?: string): Promise<Station> {
		return this.#call('/v1/me/stations', 'POST', { name, spec, ...(playerId ? { player: player(playerId) } : {}) });
	}

	deleteStation(id: number): Promise<{ removed: boolean }> {
		return this.#call(`/v1/me/stations/${pinId(id)}`, 'DELETE');
	}

	next(id: number, req: NextRequest): Promise<NextBatch> {
		const body: NextRequest = { player: player(req.player) };
		if (req.start !== undefined) body.start = req.start;
		if (req.count !== undefined) body.count = Math.max(1, Math.min(50, Math.floor(req.count)));
		if (req.exclude?.length) body.exclude = req.exclude.filter((x) => Number.isSafeInteger(x) && x > 0).slice(0, 200);
		return this.#call(`/v1/me/stations/${pinId(id)}/next`, 'POST', body);
	}

	// --- pinned menus --------------------------------------------------------------------

	async menus(): Promise<PinnedMenu[]> {
		return (await this.#call<{ menus?: PinnedMenu[] }>('/v1/me/menus')).menus ?? [];
	}

	createMenu(name: string, steps: Command[], playerId: string): Promise<PinnedMenu> {
		return this.#call('/v1/me/menus', 'POST', { name, steps, player: player(playerId) });
	}

	deleteMenu(id: number): Promise<{ removed: boolean }> {
		return this.#call(`/v1/me/menus/${pinId(id)}`, 'DELETE');
	}

	/** The pinned menu as LMS offers it now (SlimBrowse); the device may then use its items. */
	async openMenu<T = Record<string, unknown>>(id: number, playerId: string): Promise<T> {
		const r = await this.#call<{ result?: T }>(`/v1/me/menus/${pinId(id)}/open`, 'POST', { player: player(playerId) });
		return (r.result ?? {}) as T;
	}

	// --- smart filters (wave2-design.md §5.6) ------------------------------------------

	/** The user's filters (malformed ones from the server are left out). */
	async filters(): Promise<SmartFilter[]> {
		const r = await this.#call<{ filters?: unknown }>('/v1/me/filters');
		return (Array.isArray(r.filters) ? r.filters : []).map(filterFrom).filter((f): f is SmartFilter => f !== null).slice(0, FILTER_LIMITS.maxFilters);
	}

	/** Save a new filter; resolves to the id the gateway gave it. Throws before sending if invalid. */
	async createFilter(spec: FilterSpec): Promise<number> {
		const body = checked(spec);
		const r = await this.#call<{ id?: unknown }>('/v1/me/filters', 'POST', body);
		if (!validId(r.id)) throw new Error('The server gave no id for the new filter.');
		return r.id;
	}

	/** Replace a filter. Throws before sending if invalid. */
	async updateFilter(id: number, spec: FilterSpec): Promise<unknown> {
		const body = checked(spec);
		return this.#call(`/v1/me/filters/${pinId(id)}`, 'PUT', body);
	}

	async deleteFilter(id: number): Promise<unknown> {
		return this.#call(`/v1/me/filters/${pinId(id)}`, 'DELETE');
	}

	/** Album ids the filter gives, from `from`, at most `n` (≤ 100), and how many in all. */
	async filterAlbums(id: number, from = 0, n = 50): Promise<FilterAlbums> {
		const f = Math.max(0, Math.min(1_000_000, Math.floor(Number(from) || 0)));
		const k = Math.max(1, Math.min(FILTER_LIMITS.pageMax, Math.floor(Number(n) || 1)));
		const r = await this.#call<{ albums?: unknown; count?: unknown }>(`/v1/me/filters/${pinId(id)}/albums?from=${f}&n=${k}`);
		const albums = (Array.isArray(r.albums) ? r.albums : []).filter(validId).slice(0, k);
		const count = typeof r.count === 'number' && Number.isSafeInteger(r.count) && r.count >= 0 ? r.count : albums.length;
		return { albums, count };
	}
}

/** The body to send for a filter: cleaned, then checked against the contract. */
function checked(spec: FilterSpec): FilterSpec {
	let body: FilterSpec;
	try {
		body = cleanSpec(spec);
	} catch {
		throw new RangeError('not a filter');
	}
	const p = filterProblem(body);
	if (p) throw new RangeError(p);
	return body;
}
