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

	// --- listening stats (wave3-design.md §1) ------------------------------------------

	/**
	 * Listening stats for a period (`days-N`, `month-YYYY-MM`, `year-YYYY` or `all`) and the
	 * device's offset in minutes east of UTC. The answer is checked field by field: anything
	 * malformed is clamped or left out, ids come only from the server and names stay text.
	 */
	async stats(period: string, tz = 0): Promise<ListeningStats> {
		const q = new URLSearchParams({ period: statsPeriod(period), tz: String(statsTz(tz)) });
		return parseStats(await this.#call<unknown>(`/v1/me/stats?${q}`));
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

// --- listening stats -------------------------------------------------------------------

export interface StatsTotals {
	plays: number;
	seconds: number;
	tracks: number;
	albums: number;
	artists: number;
	days: number;
}
export interface StatsArtist {
	id: number;
	/** Plain text, as the gateway cleaned it ('' when it had none). */
	name: string;
	plays: number;
	seconds: number;
}
export interface StatsItem {
	id: number;
	plays: number;
	seconds: number;
}
export interface StatsNewArtist {
	id: number;
	name: string;
	/** Unix seconds, or null when the answer had none. */
	first_played: number | null;
}
export interface StatsMonth {
	/** "YYYY-MM". */
	month: string;
	plays: number;
	seconds: number;
}
export interface ListeningStats {
	enabled: boolean;
	retention_days: number;
	period: { from: number; to: number } | null;
	totals: StatsTotals;
	top_artists: StatsArtist[];
	top_albums: StatsItem[];
	top_tracks: StatsItem[];
	new_artists: StatsNewArtist[];
	/** 24 entries, midnight first, in the device's time. */
	by_hour: number[];
	/** 7 entries, Monday first. */
	by_weekday: number[];
	by_month: StatsMonth[];
	longest_streak: { days: number; from: string; to: string } | null;
	busiest_day: { date: string; seconds: number } | null;
}

/** The contract's limits on the answer and the request. */
export const STATS_LIMITS = { list: 20, months: 600, nameMax: 200, daysMin: 7, daysMax: 400, tzMax: 840 } as const;

const PERIOD = /^(?:days-([1-9]\d{0,2})|month-(\d{4})-(?:0[1-9]|1[0-2])|year-(\d{4})|all)$/;
const MONTH = /^\d{4}-(?:0[1-9]|1[0-2])$/;
const DATE = /^(\d{4})-(0[1-9]|1[0-2])-(0[1-9]|[12]\d|3[01])$/;
/** Control characters and bidi overrides: never shown as part of a name. */
// eslint-disable-next-line no-control-regex
const UNSHOWN = /[\u0000-\u001f\u007f-\u009f‪-‮⁦-⁩]/g;

/** A period the gateway accepts, or a RangeError (nothing else is ever sent). */
export function statsPeriod(v: unknown): string {
	if (typeof v !== 'string') throw new RangeError('bad period');
	const m = v.match(PERIOD);
	if (!m) throw new RangeError('bad period');
	if (m[1] !== undefined) {
		const n = Number(m[1]);
		if (n < STATS_LIMITS.daysMin || n > STATS_LIMITS.daysMax) throw new RangeError('bad period');
	}
	const y = m[2] ?? m[3];
	if (y !== undefined && Number(y) < 1970) throw new RangeError('bad period');
	return v;
}

/** A tz offset the gateway accepts: whole minutes east of UTC, −840 to 840. */
export function statsTz(v: unknown): number {
	if (typeof v !== 'number' || !Number.isInteger(v) || Math.abs(v) > STATS_LIMITS.tzMax) throw new RangeError('bad tz');
	return v === 0 ? 0 : v;
}

const count = (v: unknown, max = 1e12): number => (typeof v === 'number' && Number.isFinite(v) && v >= 0 ? Math.min(Math.floor(v), max) : 0);
const goodId = (v: unknown): v is number => typeof v === 'number' && Number.isSafeInteger(v) && v > 0;
const obj = (v: unknown): Record<string, unknown> | null => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : null);
const list = (v: unknown): unknown[] => (Array.isArray(v) ? v : []);

/** A name from the server as plain text: no control characters, bounded length. */
export function statsName(v: unknown): string {
	if (typeof v !== 'string') return '';
	const t = v.replace(UNSHOWN, '').trim();
	return t.length > STATS_LIMITS.nameMax ? `${t.slice(0, STATS_LIMITS.nameMax - 1)}…` : t;
}

/** A real calendar date "YYYY-MM-DD", or null. */
export function statsDate(v: unknown): string | null {
	if (typeof v !== 'string') return null;
	const m = v.match(DATE);
	if (!m) return null;
	const d = new Date(Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3])));
	return d.getUTCDate() === Number(m[3]) ? v : null;
}

/** Entries with a good id, each id once, at most the contract's 20. */
function byId<T extends { id: number }>(v: unknown, make: (o: Record<string, unknown>, id: number) => T): T[] {
	const out: T[] = [];
	const seen = new Set<number>();
	for (const x of list(v)) {
		const o = obj(x);
		if (!o || !goodId(o.id) || seen.has(o.id)) continue;
		seen.add(o.id);
		out.push(make(o, o.id));
		if (out.length >= STATS_LIMITS.list) break;
	}
	return out;
}

function buckets(v: unknown, n: number): number[] {
	const a = list(v);
	return Array.from({ length: n }, (_, i) => count(a[i]));
}

/** The empty answer (history off, or nothing kept). */
export function emptyStats(enabled = false, retention = 0): ListeningStats {
	return {
		enabled,
		retention_days: retention,
		period: null,
		totals: { plays: 0, seconds: 0, tracks: 0, albums: 0, artists: 0, days: 0 },
		top_artists: [],
		top_albums: [],
		top_tracks: [],
		new_artists: [],
		by_hour: Array(24).fill(0),
		by_weekday: Array(7).fill(0),
		by_month: [],
		longest_streak: null,
		busiest_day: null,
	};
}

/** The gateway's answer, checked (see `MeApi.stats`). Throws only when it isn't an object. */
export function parseStats(raw: unknown): ListeningStats {
	const r = obj(raw);
	if (!r) throw new Error('The server sent listening stats this app can’t read.');
	const retention = count(r.retention_days, 36500);
	// Switched off: nothing else in the answer is shown, whatever it says.
	if (r.enabled !== true) return emptyStats(false, retention);
	const out = emptyStats(true, retention);
	const p = obj(r.period);
	if (p && Number.isSafeInteger(p.from) && Number.isSafeInteger(p.to) && (p.from as number) >= 0 && (p.to as number) >= (p.from as number)) {
		out.period = { from: p.from as number, to: p.to as number };
	}
	const t = obj(r.totals) ?? {};
	out.totals = {
		plays: count(t.plays),
		seconds: count(t.seconds),
		tracks: count(t.tracks),
		albums: count(t.albums),
		artists: count(t.artists),
		days: count(t.days, 100000),
	};
	out.top_artists = byId(r.top_artists, (o, id) => ({ id, name: statsName(o.name), plays: count(o.plays), seconds: count(o.seconds) }));
	const item = (o: Record<string, unknown>, id: number): StatsItem => ({ id, plays: count(o.plays), seconds: count(o.seconds) });
	out.top_albums = byId(r.top_albums, item);
	out.top_tracks = byId(r.top_tracks, item);
	out.new_artists = byId(r.new_artists, (o, id) => ({
		id,
		name: statsName(o.name),
		first_played: typeof o.first_played === 'number' && Number.isSafeInteger(o.first_played) && o.first_played >= 0 ? o.first_played : null,
	}));
	out.by_hour = buckets(r.by_hour, 24);
	out.by_weekday = buckets(r.by_weekday, 7);
	const months = new Set<string>();
	for (const x of list(r.by_month)) {
		const o = obj(x);
		if (!o || typeof o.month !== 'string' || !MONTH.test(o.month) || months.has(o.month)) continue;
		months.add(o.month);
		out.by_month.push({ month: o.month, plays: count(o.plays), seconds: count(o.seconds) });
		if (out.by_month.length >= STATS_LIMITS.months) break;
	}
	out.by_month.sort((a, b) => (a.month < b.month ? -1 : 1));
	const s = obj(r.longest_streak);
	const from = statsDate(s?.from);
	const to = statsDate(s?.to);
	const days = count(s?.days, 100000);
	if (from && to && from <= to && days >= 1) out.longest_streak = { days, from, to };
	const b = obj(r.busiest_day);
	const date = statsDate(b?.date);
	if (b && date) out.busiest_day = { date, seconds: count(b.seconds, 2 * 86400) };
	return out;
}
