// The download manager (SPEC §8.3): a queue that fetches two tracks at a time, shows
// progress per track and per album or playlist, pauses, cancels, retries with backoff, and
// resumes an interrupted download from the bytes it already stored (HTTP Range).
//
// Each downloaded track lists the downloads that need it (its refs: an album, a playlist,
// or the track on its own). Removing a download takes its ref off; a track is deleted only
// when no ref is left, so a playlist never loses tracks to an album being removed.
//
// "Download on Wi-Fi only" holds the queue while the browser says it is on mobile data
// (unless the user said "Download now"), and the storage limit is checked before a batch is
// queued (by estimate) and again as each track starts (by its real size).
import { ApiError, CLIENT_HEADER, bearer } from '#lib/api/http';
import type { OfflineStore } from './store';
import {
	BadInfo,
	TRACK_REF,
	TRANSCODE_TYPES,
	audioType,
	cleanRefs,
	estimateBytes,
	isRef,
	refsOf,
	downloadUrl,
	infoUrl,
	isFormat,
	parseInfo,
	pickFormat,
	trackId,
	type Format,
	type PartialDownload,
	type QueuedDownload,
	type Ref,
	type StoredGroup,
	type StoredTrack,
	type TrackInfo,
} from './types';

export const CONCURRENCY = 2;
/** Audio is stored in pieces of about this size as it arrives. */
export const CHUNK_BYTES = 1 << 20;
/** Give up after this many failed attempts in a row (the user can retry). */
export const MAX_ATTEMPTS = 5;
/** …and after this many "too many downloads, wait" answers. */
export const MAX_WAITS = 20;
const MAX_WAIT_MS = 10 * 60 * 1000;

/** An HTTP failure, with the server's Retry-After (ms) when it sent one. */
export class HttpError extends ApiError {
	constructor(
		status: number,
		message: string,
		readonly retryAfterMs?: number,
	) {
		super(status, message);
		this.name = 'HttpError';
	}
}

/** Retry-After in ms: seconds, or an HTTP date. Capped; undefined if missing or nonsense. */
export function retryAfter(v: string | null, now = Date.now()): number | undefined {
	if (!v) return undefined;
	let ms: number;
	if (/^\d{1,9}$/.test(v.trim())) ms = Number(v.trim()) * 1000;
	else {
		const t = Date.parse(v);
		if (!Number.isFinite(t)) return undefined;
		ms = t - now;
	}
	return Math.min(MAX_WAIT_MS, Math.max(1000, ms));
}

export type Authed = <T>(fn: (token: string) => Promise<T>) => Promise<T>;

export interface ManagerDeps {
	store: OfflineStore;
	/** Runs a call with the access token, renewing it once on a 401 (session.authed). */
	authed: Authed;
	fetch?: typeof fetch;
	/** The album cover for a track, as an image Blob (fetched once per album). */
	cover?: (trackId: number) => Promise<Blob | null>;
	/** The format to use when the user didn't pick one (by network type). */
	defaultFormat?: () => Format;
	sleep?: (ms: number, signal: AbortSignal) => Promise<void>;
	now?: () => number;
	/** Ask the browser to keep downloads (navigator.storage.persist). */
	persist?: () => Promise<unknown>;
	/** True while downloads must wait for Wi-Fi (Wi-Fi only is on and this is mobile data). */
	waitForWifi?: () => boolean;
	/** The storage limit for downloads in bytes (0: none). */
	capBytes?: () => number;
	/** The browser's storage estimate (navigator.storage.estimate). */
	storageEstimate?: () => Promise<{ usage?: number; quota?: number }>;
}

/** A track to download, as a library list gives it (sizes are for the storage limit). */
export interface DownloadRequest {
	id: unknown;
	albumId?: unknown;
	duration?: unknown;
	filesize?: unknown;
	/** What it is downloaded as part of (else the request's `ref`). */
	ref?: Ref;
}

export interface RequestOptions {
	choice?: Format | 'auto';
	/** What these tracks are downloaded as part of: `album:<id>`, `playlist:<id>` or `track`. */
	ref?: Ref;
	/** For a playlist: its name and order, shown on the Downloads page. */
	group?: { name: string; ids: number[] };
}

export interface RequestResult {
	/** Tracks added to the queue. */
	queued: number;
	/** Why nothing was queued (the storage limit or the browser's space), for the user. */
	refused?: string;
}

/** What a removal did: the tracks deleted, and how many stayed for other downloads. */
export interface Released {
	removed: string[];
	kept: number;
}

/** A track would take downloads past the user's storage limit. */
export class LimitReached extends Error {
	constructor() {
		super('storage limit reached');
		this.name = 'LimitReached';
	}
}

/**
 * The answer is not audio: a proxy, captive portal or misrouted request answered 200 with a
 * web page or JSON. Such bytes are never stored as a track.
 */
export class NotAudio extends Error {
	constructor(what: string) {
		super(`not audio (${what})`);
		this.name = 'NotAudio';
	}
}
export const NOT_AUDIO_MESSAGE = 'The server sent something that isn’t audio.';

/** Content types that are certainly not a track (audio, generic binary or none are let through). */
export function notAudioType(v: string | null): boolean {
	const t = (v ?? '').split(';')[0]!.trim().toLowerCase();
	return /^(text|image|font|multipart)\//.test(t) || /^application\/(json|[\w.-]+\+json|xml|[\w.-]+\+xml|javascript|ecmascript|x-www-form-urlencoded|pdf)$/.test(t);
}

/** Whether the first bytes are a web page, XML or JSON rather than any audio container. */
export function looksLikeMarkup(head: Uint8Array): boolean {
	let i = 0;
	if (head[0] === 0xef && head[1] === 0xbb && head[2] === 0xbf) i = 3; // a UTF-8 byte-order mark
	while (i < head.length && i < 64 && (head[i] === 0x20 || head[i] === 0x09 || head[i] === 0x0a || head[i] === 0x0d)) i++;
	const text = String.fromCharCode(...head.subarray(i, i + 16)).toLowerCase();
	return /^(<!doctype|<html|<head|<body|<\?xml|<!--|<script|<meta|<title)/.test(text) || /^\{\s*"/.test(text) || /^\[\s*[{"]/.test(text);
}

export const LIMIT_MESSAGE = 'Storage limit reached. Remove some downloads, or raise the limit in Settings.';

/** "23.4 MB", "1.20 GB" (as settings' formatBytes; kept here so the manager stands alone). */
function formatSize(n: number): string {
	if (!Number.isFinite(n) || n <= 0) return '0 MB';
	if (n >= 1024 ** 3) return `${(n / 1024 ** 3).toFixed(2)} GB`;
	return `${(n / 1024 ** 2).toFixed(1)} MB`;
}

export type ItemStatus = 'queued' | 'active' | 'waiting' | 'paused' | 'done' | 'failed';

export interface DownloadItem {
	id: number;
	albumId: number | null;
	/** The downloads that asked for this track. */
	refs: Ref[];
	/** "Download now": may use mobile data although Wi-Fi only is on. */
	cellular: boolean;
	/** Expected size before the server says (for the storage limit). */
	estimate: number;
	choice: Format | 'auto';
	status: ItemStatus;
	title: string;
	key: string;
	format: Format | null;
	received: number;
	total: number;
	attempts: number;
	waits: number;
	/** Why it failed, or why it is waiting. */
	message: string;
}

export interface AlbumProgress {
	count: number;
	done: number;
	failed: number;
	paused: number;
	/** Anything queued, downloading or waiting. */
	busy: boolean;
	/** 0…1 across the album's tracks. */
	fraction: number;
}

const abortError = () => new DOMException('aborted', 'AbortError');

function defaultSleep(ms: number, signal: AbortSignal): Promise<void> {
	return new Promise((resolve, reject) => {
		if (signal.aborted) return reject(abortError());
		const t = setTimeout(resolve, ms);
		signal.addEventListener(
			'abort',
			() => {
				clearTimeout(t);
				reject(abortError());
			},
			{ once: true },
		);
	});
}

/** What to do about a failure: give up (with a message for the user), or try again. */
export function classify(e: unknown): { permanent: boolean; message: string; waitMs?: number; counts?: boolean } {
	if (e instanceof BadInfo) return { permanent: true, message: 'The server sent unexpected track details.' };
	if (e instanceof ApiError) {
		const s = e.status;
		if (s === 404) return { permanent: true, message: 'Not available for download (only tracks in your library can be downloaded).' };
		if (s === 403) return { permanent: true, message: 'Your account isn’t allowed to download.' };
		if (s === 401) return { permanent: true, message: 'Signed out. Sign in again to download.' };
		if (s === 429) {
			const waitMs = e instanceof HttpError ? e.retryAfterMs : undefined;
			return { permanent: false, message: 'Waiting: the server is busy with other downloads.', ...(waitMs !== undefined ? { waitMs } : {}), counts: false };
		}
		if (s === 416) return { permanent: false, message: 'Starting again.' };
		if (s >= 400 && s < 500 && s !== 408) return { permanent: true, message: `The server refused the download (HTTP ${s}).` };
		return { permanent: false, message: `Server error (HTTP ${s}). Retrying…` };
	}
	if (e instanceof LimitReached) return { permanent: true, message: LIMIT_MESSAGE };
	if (e instanceof NotAudio) return { permanent: true, message: NOT_AUDIO_MESSAGE };
	if (e instanceof DOMException && e.name === 'QuotaExceededError') {
		return { permanent: true, message: 'Not enough storage space on this device.' };
	}
	return { permanent: false, message: 'Connection lost. Retrying…' };
}

/** Exponential backoff: 2 s, 4 s, 8 s… up to a minute. */
export const backoff = (attempt: number) => Math.min(60_000, 1000 * 2 ** attempt);

async function fail(resp: Response): Promise<never> {
	throw new HttpError(resp.status, `HTTP ${resp.status}`, resp.status === 429 ? retryAfter(resp.headers.get('retry-after')) : undefined);
}

export async function fetchInfo(f: typeof fetch, id: number, token: string, signal?: AbortSignal): Promise<TrackInfo> {
	const resp = await f(infoUrl(id), { headers: { ...CLIENT_HEADER, ...bearer(token) }, credentials: 'same-origin', cache: 'no-store', ...(signal ? { signal } : {}) });
	if (!resp.ok) await fail(resp);
	let body: unknown;
	try {
		body = await resp.json();
	} catch {
		throw new BadInfo('not JSON');
	}
	return parseInfo(body, id);
}

export async function fetchAudio(f: typeof fetch, id: number, format: Format, token: string, offset: number, signal?: AbortSignal): Promise<Response> {
	const headers: Record<string, string> = { ...CLIENT_HEADER, ...bearer(token) };
	if (offset > 0) headers.Range = `bytes=${offset}-`;
	const resp = await f(downloadUrl(id, format), { headers, credentials: 'same-origin', cache: 'no-store', ...(signal ? { signal } : {}) });
	if (!resp.ok) await fail(resp);
	return resp;
}

/** "bytes 100-199/200" → start and total (total 0 if "*"). */
export function contentRange(v: string | null): { start: number; total: number } | null {
	const m = (v ?? '').trim().match(/^bytes (\d{1,15})-(\d{1,15})\/(\d{1,15}|\*)$/);
	if (!m) return null;
	return { start: Number(m[1]), total: m[3] === '*' ? 0 : Number(m[3]) };
}

export class DownloadManager {
	items = $state<DownloadItem[]>([]);
	/** This user's downloaded tracks. */
	saved = $state<StoredTrack[]>([]);
	user = $state<string | null>(null);
	/** False in offline mode: show downloads, but fetch nothing. */
	online = $state(false);
	/** Downloaded playlists (name and order) for this user. */
	groups = $state<StoredGroup[]>([]);
	/** Set if storage can't be used at all. */
	error = $state('');
	/** The queue is held: Wi-Fi only is on and the browser says this is mobile data. */
	holding = $state(false);

	#ctl = new Map<number, AbortController>();
	#runs = new Map<number, Promise<void>>();
	#active = 0;
	/** Bumped when the user changes: work started for the previous one must stop. */
	#gen = 0;
	#fetch: typeof fetch;
	#sleep: (ms: number, signal: AbortSignal) => Promise<void>;

	constructor(private deps: ManagerDeps) {
		this.#fetch = deps.fetch ?? ((...a) => fetch(...a));
		this.#sleep = deps.sleep ?? defaultSleep;
	}

	get store(): OfflineStore {
		return this.deps.store;
	}

	/**
	 * Switch to a user's downloads (null when signed out). `online` false shows them without
	 * fetching anything: offline mode, before signing in.
	 */
	async setUser(user: string | null, online = true): Promise<void> {
		if (user === this.user) {
			const was = this.online;
			this.online = online && !!user;
			if (this.online && !was) await this.#restoreQueue(this.#gen);
			return;
		}
		this.#stop();
		const gen = ++this.#gen;
		this.user = user;
		this.online = online && !!user;
		this.items = [];
		this.saved = [];
		this.groups = [];
		this.error = '';
		if (!user) return;
		try {
			const saved = await this.store.listTracks(user);
			const groups = await this.store.listGroups(user).catch(() => [] as StoredGroup[]);
			if (gen !== this.#gen) return;
			this.saved = saved;
			this.groups = groups.filter((g) => isRef(g?.id) && g.id !== TRACK_REF && typeof g.name === 'string' && Array.isArray(g.ids));
			if (this.online) await this.#restoreQueue(gen);
		} catch (e) {
			if (gen === this.#gen) this.error = (e as Error).message;
		}
	}

	async #restoreQueue(gen: number) {
		const user = this.user;
		if (!user) return;
		const q = await this.store.loadQueue(user).catch(() => [] as QueuedDownload[]);
		if (gen !== this.#gen) return;
		for (const e of q) {
			try {
				const albumId = typeof e.albumId === 'number' && Number.isSafeInteger(e.albumId) && e.albumId > 0 ? e.albumId : null;
				const refs = cleanRefs(e.refs);
				this.#add(trackId(e.id), albumId, e.choice === 'auto' || isFormat(e.choice) ? e.choice : 'auto', e.paused ? 'paused' : 'queued', {
					refs: refs.length ? refs : [albumId !== null ? `album:${albumId}` : TRACK_REF],
					cellular: e.cellular === true,
					estimate: typeof e.estimate === 'number' && Number.isSafeInteger(e.estimate) && e.estimate > 0 ? e.estimate : 0,
				});
				// Its stable key, once known: a cancel before the details come back still finds its bytes.
				const it = this.item(e.id);
				if (it && !it.key && typeof e.key === 'string' && /^[0-9a-f]{64}$/.test(e.key)) it.key = e.key;
			} catch {
				/* a bad entry: drop it */
			}
		}
		this.#pump();
	}

	/** Abort everything in flight (keeping what was stored, to resume later). */
	#stop() {
		for (const c of this.#ctl.values()) c.abort();
		this.#ctl.clear();
	}

	item(id: number): DownloadItem | undefined {
		return this.items.find((i) => i.id === id);
	}

	/** The stored copy of an LMS track id, if this device has it. */
	savedById(id: unknown): StoredTrack | undefined {
		return typeof id === 'number' ? this.saved.find((t) => t.id === id) : undefined;
	}

	savedAlbum(albumId: number): StoredTrack[] {
		return this.saved.filter((t) => t.albumId === albumId);
	}

	#add(
		id: number,
		albumId: number | null,
		choice: Format | 'auto',
		status: ItemStatus = 'queued',
		extra: { refs: Ref[]; cellular?: boolean; estimate?: number } = { refs: [albumId !== null ? `album:${albumId}` : TRACK_REF] },
	): DownloadItem {
		const existing = this.item(id);
		if (existing) {
			existing.refs = cleanRefs([...existing.refs, ...extra.refs]);
			if (existing.status === 'failed' || existing.status === 'paused' || existing.status === 'done') {
				Object.assign(existing, { status, choice, attempts: 0, waits: 0, message: '' });
				if (extra.estimate) existing.estimate = extra.estimate;
			}
			if (extra.cellular) existing.cellular = true;
			return existing;
		}
		this.items.push({
			id,
			albumId,
			refs: cleanRefs(extra.refs),
			cellular: extra.cellular === true,
			estimate: extra.estimate ?? 0,
			choice,
			status,
			title: '',
			key: '',
			format: null,
			received: 0,
			total: 0,
			attempts: 0,
			waits: 0,
			message: '',
		});
		return this.items[this.items.length - 1]!;
	}

	/** Bytes the queue still expects to store (what is downloading or waiting). */
	get pendingBytes(): number {
		let n = 0;
		for (const i of this.items) if (i.status !== 'done' && i.status !== 'failed') n += Math.max(i.total, i.estimate, 0);
		return n;
	}

	/**
	 * Download tracks as part of an album, a playlist or on their own. Tracks already on the
	 * device just gain the ref (nothing is fetched); the others are queued, unless they would
	 * take downloads past the storage limit or the browser's free space, in which case
	 * nothing is queued and `refused` says why. Ids must be LMS track ids; anything else is
	 * dropped.
	 */
	async request(tracks: readonly DownloadRequest[], opts: RequestOptions = {}): Promise<RequestResult> {
		const user = this.user;
		if (!user || !this.online) return { queued: 0 };
		const gen = this.#gen;
		const choice = opts.choice ?? 'auto';
		const format = choice === 'auto' ? (this.deps.defaultFormat?.() ?? 'original') : choice;
		const fallback = opts.ref !== undefined && isRef(opts.ref) ? opts.ref : TRACK_REF;
		const adopt = new Map<string, Ref[]>();
		const fresh: { id: number; albumId: number | null; ref: Ref; estimate: number }[] = [];
		const seen = new Set<number>();
		let need = 0;
		for (const t of tracks) {
			let id: number;
			try {
				id = trackId(t.id);
			} catch {
				continue;
			}
			if (seen.has(id)) continue;
			seen.add(id);
			const ref = t.ref !== undefined && isRef(t.ref) ? t.ref : fallback;
			const have = this.savedById(id);
			if (have) {
				adopt.set(have.key, [...(adopt.get(have.key) ?? []), ref]);
				continue;
			}
			const albumId = typeof t.albumId === 'number' && Number.isSafeInteger(t.albumId) && t.albumId > 0 ? t.albumId : null;
			const it = this.item(id);
			const estimate = estimateBytes(t, format);
			// Already waiting or downloading: it gains the ref, and its bytes are counted already.
			if (!it || it.status === 'done' || it.status === 'failed') need += estimate;
			fresh.push({ id, albumId, ref, estimate });
		}
		const refused = await this.#room(need);
		if (gen !== this.#gen || !this.online) return { queued: 0 };
		if (refused) return { queued: 0, refused };

		if (opts.group && fallback !== TRACK_REF) {
			const ids = opts.group.ids.filter((i) => Number.isSafeInteger(i) && i > 0).slice(0, 5000);
			const g: StoredGroup = { user, id: fallback, name: String(opts.group.name ?? '').slice(0, 300), ids, savedAt: this.deps.now?.() ?? Date.now() };
			await this.store.putGroup(g).catch(() => {});
			if (gen !== this.#gen) return { queued: 0 };
			this.groups = [...this.groups.filter((x) => x.id !== g.id), g];
		}
		for (const [key, refs] of adopt) await this.#addRefs(user, key, refs, gen);
		let n = 0;
		for (const f of fresh) {
			const it = this.#add(f.id, f.albumId, choice, 'queued', { refs: [f.ref], estimate: f.estimate });
			if (it.status === 'queued') n++;
		}
		if (n) void this.deps.persist?.();
		this.#saveQueue();
		this.#pump();
		return { queued: n };
	}

	/** Why `need` more bytes can't be stored now, or '' if they can. */
	async #room(need: number): Promise<string> {
		if (need <= 0) return '';
		const cap = this.deps.capBytes?.() ?? 0;
		if (Number.isFinite(cap) && cap > 0) {
			const free = Math.max(0, cap - this.usedBytes - this.pendingBytes);
			if (need > free) {
				return `Not enough room under your storage limit: this needs about ${formatSize(need)}, and ${formatSize(free)} of ${formatSize(cap)} is free. Remove some downloads, or raise the limit in Settings.`;
			}
		}
		let est: { usage?: number; quota?: number } | undefined;
		try {
			est = await this.deps.storageEstimate?.();
		} catch {
			est = undefined; // unknown: don't stand in the way
		}
		const quota = est?.quota;
		const usage = est?.usage ?? 0;
		if (typeof quota === 'number' && Number.isFinite(quota) && quota > 0 && Number.isFinite(usage)) {
			// What is still to arrive for downloads already queued (their stored bytes are in `usage`).
			let coming = 0;
			for (const i of this.items) if (i.status !== 'done' && i.status !== 'failed') coming += Math.max(0, Math.max(i.total, i.estimate) - i.received);
			const free = Math.max(0, quota - usage - coming);
			if (need > free) return `Not enough space in this browser: this needs about ${formatSize(need)}, and only ${formatSize(free)} is free.`;
		}
		return '';
	}

	/** Add refs to a stored track (it is needed by one more download). */
	async #addRefs(user: string, key: string, refs: Ref[], gen: number) {
		const t = this.saved.find((s) => s.key === key);
		if (!t) return;
		const merged = cleanRefs([...refsOf(t), ...refs]);
		if (Array.isArray(t.refs) && merged.length === t.refs.length) return;
		const next = { ...t, refs: merged };
		try {
			await this.store.putTrack(next);
		} catch {
			return;
		}
		this.#saved(gen, next);
	}

	/**
	 * Download tracks (in order), each as part of its album (or on its own without one),
	 * with no storage check: [`request`] is what the pages use. Tracks already on the device
	 * are checked by their stable key and not fetched again. Ids must be LMS track ids;
	 * anything else is refused.
	 */
	enqueue(tracks: { id: unknown; albumId?: unknown }[], choice: Format | 'auto' = 'auto'): number {
		if (!this.user || !this.online) return 0;
		let n = 0;
		for (const t of tracks) {
			let id: number;
			try {
				id = trackId(t.id);
			} catch {
				continue;
			}
			if (this.savedById(id)) continue; // already on this device
			const albumId = typeof t.albumId === 'number' && Number.isSafeInteger(t.albumId) && t.albumId > 0 ? t.albumId : null;
			const it = this.#add(id, albumId, choice);
			if (it.status === 'queued') n++;
		}
		if (n) void this.deps.persist?.();
		this.#saveQueue();
		this.#pump();
		return n;
	}

	#saveQueue() {
		const user = this.user;
		if (!user) return;
		const q: QueuedDownload[] = this.items
			.filter((i) => i.status !== 'done' && i.status !== 'failed')
			.map((i) => ({ id: i.id, albumId: i.albumId, choice: i.choice, paused: i.status === 'paused', refs: [...i.refs], cellular: i.cellular, estimate: i.estimate, ...(i.key ? { key: i.key } : {}) }));
		void this.store.saveQueue(user, q).catch(() => {});
	}

	#holdNow(): boolean {
		try {
			return this.deps.waitForWifi?.() === true;
		} catch {
			return false;
		}
	}

	/** Waiting for Wi-Fi: queued, and held because this is mobile data. */
	isHeld(i: DownloadItem): boolean {
		return this.holding && !i.cellular && i.status === 'queued';
	}

	/**
	 * The network or the Wi-Fi only setting changed. On mobile data with Wi-Fi only on,
	 * downloads in progress stop (keeping their bytes, to resume by Range) and wait;
	 * on Wi-Fi they carry on.
	 */
	networkChanged() {
		this.holding = this.#holdNow();
		if (this.holding) {
			for (const i of this.items) {
				if (i.cellular || (i.status !== 'active' && i.status !== 'waiting')) continue;
				i.status = 'queued';
				i.message = '';
				this.#ctl.get(i.id)?.abort();
			}
		}
		this.#pump();
	}

	/** "Download now": let these waiting downloads (all, or those matching) use mobile data. */
	downloadNow(match: (i: DownloadItem) => boolean = () => true) {
		let any = false;
		for (const i of this.items) {
			if (i.status === 'done' || i.status === 'failed' || i.cellular || !match(i)) continue;
			i.cellular = true;
			any = true;
		}
		if (!any) return;
		this.#saveQueue();
		this.#pump();
	}

	#pump() {
		if (!this.online) return;
		this.holding = this.#holdNow();
		while (this.#active < CONCURRENCY) {
			const next = this.items.find((i) => i.status === 'queued' && (!this.holding || i.cellular));
			if (!next) return;
			this.#active++;
			next.status = 'active';
			const run: Promise<void> = this.#run(next).finally(() => {
				this.#active--;
				if (this.#runs.get(next.id) === run) this.#runs.delete(next.id);
				this.#saveQueue();
				this.#pump();
			});
			this.#runs.set(next.id, run);
		}
	}

	async #run(item: DownloadItem): Promise<void> {
		const gen = this.#gen;
		const user = this.user!;
		const ctl = new AbortController();
		this.#ctl.set(item.id, ctl);
		const stale = () => ctl.signal.aborted || gen !== this.#gen;
		try {
			for (;;) {
				try {
					item.status = 'active';
					await this.#attempt(item, user, ctl.signal, gen);
					if (stale()) return;
					item.status = 'done';
					item.message = '';
					return;
				} catch (e) {
					if (stale()) return; // paused, cancelled or signed out: they set the status
					const d = classify(e);
					if (!d.permanent) {
						if (d.counts === false) item.waits++;
						else item.attempts++;
					}
					if (d.permanent || item.attempts >= MAX_ATTEMPTS || item.waits > MAX_WAITS) {
						item.status = 'failed';
						item.message = d.permanent ? d.message : 'Download failed. Try again later.';
						return;
					}
					item.status = 'waiting';
					item.message = d.message;
					try {
						await this.#sleep(d.waitMs ?? backoff(item.attempts), ctl.signal);
					} catch {
						return; // paused or cancelled while waiting
					}
					if (stale()) return;
				}
			}
		} finally {
			if (this.#ctl.get(item.id) === ctl) this.#ctl.delete(item.id);
		}
	}

	async #attempt(item: DownloadItem, user: string, signal: AbortSignal, gen: number): Promise<void> {
		const store = this.store;
		const f = this.#fetch;
		const info = await this.deps.authed((t) => fetchInfo(f, item.id, t, signal));
		if (signal.aborted) throw abortError();
		item.key = info.key;
		item.title = info.title;
		item.albumId ??= info.albumId;
		const want = item.choice === 'auto' ? (this.deps.defaultFormat?.() ?? 'original') : item.choice;
		const format = pickFormat(want, info.formats);
		item.format = format;
		const cap = this.deps.capBytes?.() ?? 0;
		const limited = Number.isFinite(cap) && cap > 0;
		const base = this.usedBytes;

		// Already here (perhaps under an older track id, before a rescan): just refresh the id.
		const have = await store.getTrack(user, info.key);
		if (have) {
			const refs = cleanRefs([...refsOf(have), ...item.refs]);
			const t = { ...have, id: info.id, albumId: info.albumId ?? have.albumId, refs };
			if (t.id !== have.id || t.albumId !== have.albumId || refs.length !== (have.refs?.length ?? -1)) await store.putTrack(t);
			this.#saved(gen, t);
			item.received = item.total = have.size;
			return;
		}

		let partial = await store.getPartial(user, info.key);
		if (partial && partial.format !== format) {
			await store.discardPartial(user, info.key);
			partial = undefined;
		}
		let offset = partial?.received ?? 0;
		item.total = format === 'original' ? info.original.size : (partial?.total ?? 0);
		item.received = offset;
		// The storage limit, by the real size where known (an estimate for a transcode), with
		// what the other downloads in progress have stored so far.
		const expected = format === 'original' && info.original.size ? info.original.size : estimateBytes({ duration: info.duration }, format);
		const others = this.items.reduce((n, i) => (i !== item && i.status === 'active' ? n + i.received : n), 0);
		if (limited && base + others + expected > cap) throw new LimitReached();

		const resp = await this.deps.authed((t) => fetchAudio(f, item.id, format, t, offset, signal));
		let seq = partial?.chunks ?? 0;
		let total = 0;
		const length = Number(resp.headers.get('content-length'));
		const known = (n: number) => Number.isSafeInteger(n) && n > 0;
		if (offset > 0 && resp.status === 206) {
			const range = contentRange(resp.headers.get('content-range'));
			if (!range || range.start !== offset) {
				await resp.body?.cancel().catch(() => {});
				await store.discardPartial(user, info.key);
				throw new HttpError(416, 'range mismatch');
			}
			total = range.total || (known(length) ? offset + length : 0);
		} else {
			// A whole file (the server ignored the Range, or there was nothing to resume).
			if (offset > 0) await store.discardPartial(user, info.key);
			offset = 0;
			seq = 0;
			total = known(length) ? length : 0;
		}
		if (notAudioType(resp.headers.get('content-type'))) {
			await resp.body?.cancel().catch(() => {});
			await store.discardPartial(user, info.key);
			throw new NotAudio(resp.headers.get('content-type') ?? '');
		}
		if (format === 'original' && info.original.size && total && total !== info.original.size) {
			await resp.body?.cancel().catch(() => {});
			await store.discardPartial(user, info.key);
			throw new HttpError(502, 'size mismatch');
		}
		const contentType =
			format === 'original'
				? info.original.contentType || audioType(resp.headers.get('content-type')) || 'application/octet-stream'
				: TRANSCODE_TYPES[format].type;
		item.total = total;
		item.received = offset;

		let p: PartialDownload = { user, key: info.key, format, total, received: offset, chunks: seq, contentType };
		await store.putPartial(p);

		let buf: Uint8Array[] = [];
		let bufLen = 0;
		const flush = async () => {
			if (!bufLen) return;
			await store.putChunk(user, info.key, seq, new Blob(buf as BlobPart[]));
			seq++;
			p = { ...p, received: p.received + bufLen, chunks: seq };
			buf = [];
			bufLen = 0;
			await store.putPartial(p);
		};
		// The start of the file must not be a web page (whatever the content type claimed).
		let sniffed = offset > 0;
		const take = async (bytes: Uint8Array) => {
			if (signal.aborted) throw abortError();
			if (!sniffed) {
				sniffed = true;
				if (looksLikeMarkup(bytes)) {
					await store.discardPartial(user, info.key);
					throw new NotAudio('markup');
				}
			}
			buf.push(bytes);
			bufLen += bytes.byteLength;
			item.received += bytes.byteLength;
			if (total && item.received > total) {
				buf = [];
				bufLen = 0;
				await store.discardPartial(user, info.key);
				throw new HttpError(502, 'more data than expected');
			}
			if (limited && base + item.received > cap) {
				buf = [];
				bufLen = 0;
				await store.discardPartial(user, info.key);
				throw new LimitReached();
			}
			if (bufLen >= CHUNK_BYTES) await flush();
		};
		const reader = resp.body?.getReader();
		if (reader) {
			// Pausing or cancelling ends the read at once, even if the body would stall.
			const stop = () => void reader.cancel().catch(() => {});
			signal.addEventListener('abort', stop, { once: true });
			try {
				for (;;) {
					const { done, value } = await reader.read();
					if (done) break;
					await take(value);
				}
			} catch (e) {
				reader.cancel().catch(() => {});
				// Keep what arrived: the next attempt resumes from there.
				if (!(e instanceof HttpError) && !(e instanceof NotAudio) && gen === this.#gen) await flush().catch(() => {});
				throw e;
			} finally {
				signal.removeEventListener('abort', stop);
			}
		} else {
			await take(new Uint8Array(await resp.arrayBuffer()));
		}
		await flush();
		if (signal.aborted) throw abortError();
		if (total && p.received < total) throw new Error('connection closed early'); // resumes
		const track: StoredTrack = {
			user,
			key: info.key,
			id: info.id,
			title: info.title,
			artist: info.artist,
			album: info.album,
			albumId: info.albumId ?? item.albumId,
			tracknum: info.tracknum,
			duration: info.duration,
			format,
			codec: format === 'original' ? info.original.format : format,
			ext: format === 'original' ? info.original.ext : TRANSCODE_TYPES[format].ext,
			size: p.received,
			contentType,
			downloadedAt: this.deps.now?.() ?? Date.now(),
			chunks: seq,
			refs: cleanRefs(item.refs.length ? item.refs : [item.albumId !== null ? `album:${item.albumId}` : TRACK_REF]),
		};
		await store.putTrack(track);
		await store.finishPartial(user, info.key);
		item.total = item.received = track.size;
		await this.#cover(user, track);
		this.#saved(gen, track);
	}

	async #cover(user: string, t: StoredTrack) {
		if (t.albumId === null || !this.deps.cover) return;
		try {
			if (await this.store.getCover(user, t.albumId)) return;
			const blob = await this.deps.cover(t.id);
			if (blob && /^image\//.test(blob.type)) await this.store.putCover(user, t.albumId, blob);
		} catch {
			/* no cover: the album shows a placeholder */
		}
	}

	#saved(gen: number, t: StoredTrack) {
		if (gen !== this.#gen) return;
		this.saved = [...this.saved.filter((s) => s.key !== t.key), t];
	}

	pause(id: number) {
		const it = this.item(id);
		if (!it || (it.status !== 'queued' && it.status !== 'active' && it.status !== 'waiting')) return;
		it.status = 'paused';
		it.message = '';
		this.#ctl.get(id)?.abort();
		this.#saveQueue();
	}

	resume(id: number) {
		const it = this.item(id);
		if (!it || (it.status !== 'paused' && it.status !== 'failed')) return;
		Object.assign(it, { status: 'queued', attempts: 0, waits: 0, message: '' });
		this.#saveQueue();
		this.#pump();
	}

	/** Stop a download and throw away what it had stored. */
	async cancel(id: number): Promise<void> {
		const it = this.item(id);
		if (!it) return;
		const user = this.user;
		if (it.status !== 'done' && it.status !== 'failed') it.status = 'paused'; // the run stops without marking anything
		this.#ctl.get(id)?.abort();
		await this.#runs.get(id)?.catch(() => {});
		this.items = this.items.filter((i) => i.id !== id);
		this.#saveQueue();
		if (user && it.key && !this.saved.some((s) => s.key === it.key)) await this.store.discardPartial(user, it.key).catch(() => {});
	}


	/** Downloads of one album (by id), those matching, or all. */
	#which(sel?: number | ((i: DownloadItem) => boolean)): (i: DownloadItem) => boolean {
		if (sel === undefined) return () => true;
		if (typeof sel === 'function') return sel;
		return (i) => i.albumId === sel;
	}

	pauseAll(sel?: number | ((i: DownloadItem) => boolean)) {
		const m = this.#which(sel);
		for (const i of this.items) if (m(i)) this.pause(i.id);
	}

	resumeAll(sel?: number | ((i: DownloadItem) => boolean)) {
		const m = this.#which(sel);
		for (const i of [...this.items]) if (m(i)) this.resume(i.id);
	}

	async cancelAll(sel?: number | ((i: DownloadItem) => boolean)) {
		const m = this.#which(sel);
		await Promise.all(this.items.filter(m).map((i) => this.cancel(i.id)));
	}

	/** Forget finished and failed entries in the list of downloads (and a failed one's bytes). */
	clearFinished() {
		const user = this.user;
		for (const i of this.items) {
			if (user && i.status === 'failed' && i.key && !this.saved.some((s) => s.key === i.key)) void this.store.discardPartial(user, i.key).catch(() => {});
		}
		this.items = this.items.filter((i) => i.status !== 'done' && i.status !== 'failed');
		this.#saveQueue();
	}

	/** Progress of the downloads that match (an album's, a playlist's…). */
	progress(match: (i: DownloadItem) => boolean): AlbumProgress {
		const its = this.items.filter(match);
		let sum = 0;
		const p: AlbumProgress = { count: its.length, done: 0, failed: 0, paused: 0, busy: false, fraction: 0 };
		for (const i of its) {
			if (i.status === 'done') {
				p.done++;
				sum += 1;
			} else {
				if (i.status === 'failed') p.failed++;
				else if (i.status === 'paused') p.paused++;
				else p.busy = true;
				sum += i.total > 0 ? Math.min(1, i.received / i.total) : 0;
			}
		}
		p.fraction = its.length ? sum / its.length : 0;
		return p;
	}

	album(albumId: number): AlbumProgress {
		return this.progress((i) => i.albumId === albumId);
	}

	/** Progress of the downloads asked for as part of `ref` (e.g. `playlist:3`). */
	group(ref: Ref): AlbumProgress {
		return this.progress((i) => i.refs.includes(ref));
	}

	/** The stored tracks a download (`album:…`, `playlist:…`, `track`) needs. */
	savedFor(ref: Ref): StoredTrack[] {
		return this.saved.filter((t) => refsOf(t).includes(ref));
	}

	/**
	 * Take refs (those `drop` picks) off the tracks `track` picks, and off downloads still in
	 * the queue that `item` picks. A track left with no ref is deleted with its audio; a
	 * queued download left with none is cancelled. Nothing another download needs is lost.
	 */
	async #release(drop: (r: Ref) => boolean, track: (t: StoredTrack) => boolean, item: (i: DownloadItem) => boolean): Promise<Released> {
		const user = this.user;
		if (!user) return { removed: [], kept: 0 };
		const gen = this.#gen;
		const cancel: number[] = [];
		for (const i of this.items) {
			if (i.status === 'done' || !item(i)) continue;
			i.refs = i.refs.filter((r) => !drop(r));
			if (!i.refs.length) cancel.push(i.id);
		}
		for (const id of cancel) await this.cancel(id);
		const removed: string[] = [];
		let kept = 0;
		for (const t of this.saved.filter(track)) {
			const refs = refsOf(t).filter((r) => !drop(r));
			if (refs.length) {
				const next = { ...t, refs };
				await this.store.putTrack(next);
				this.#saved(gen, next);
				kept++;
			} else {
				await this.store.deleteTrack(user, t.key);
				removed.push(t.key);
			}
		}
		if (gen !== this.#gen) return { removed, kept };
		const gone = new Set(removed);
		this.items = this.items.filter((i) => !gone.has(i.key));
		this.saved = this.saved.filter((t) => !gone.has(t.key));
		await this.#pruneGroups(user, gen);
		this.#saveQueue();
		return { removed, kept };
	}

	/** Forget downloaded playlists that no track or queued download needs any more. */
	async #pruneGroups(user: string, gen: number) {
		const used = new Set<string>();
		for (const t of this.saved) for (const r of refsOf(t)) used.add(r);
		for (const i of this.items) if (i.status !== 'done' && i.status !== 'failed') for (const r of i.refs) used.add(r);
		for (const g of this.groups) {
			if (used.has(g.id)) continue;
			await this.store.deleteGroup(user, g.id).catch(() => {});
		}
		if (gen === this.#gen) this.groups = this.groups.filter((g) => used.has(g.id));
	}

	/** Delete downloaded tracks from this device, whatever else needs them. */
	async remove(keys: string[]): Promise<void> {
		const ks = new Set(keys);
		await this.#release(
			() => true,
			(t) => ks.has(t.key),
			(i) => ks.has(i.key),
		);
	}

	/**
	 * Remove tracks downloaded on their own or with their albums. Tracks a downloaded
	 * playlist still needs stay (counted in `kept`).
	 */
	removeTracks(keys: readonly string[]): Promise<Released> {
		const ks = new Set(keys);
		return this.#release(
			(r) => r === TRACK_REF || r.startsWith('album:'),
			(t) => ks.has(t.key),
			(i) => ks.has(i.key),
		);
	}

	/** Remove an album's download (and its tracks downloaded on their own), keeping tracks a playlist needs. */
	removeAlbum(albumId: number): Promise<Released> {
		const ref = `album:${albumId}`;
		return this.#release(
			(r) => r === ref || r === TRACK_REF,
			(t) => t.albumId === albumId || refsOf(t).includes(ref),
			(i) => i.albumId === albumId || i.refs.includes(ref),
		);
	}

	/** Remove a playlist's download, keeping tracks an album or another playlist needs. */
	async removePlaylist(playlistId: number): Promise<Released> {
		const ref = `playlist:${playlistId}`;
		const r = await this.#release(
			(x) => x === ref,
			(t) => refsOf(t).includes(ref),
			(i) => i.refs.includes(ref),
		);
		const user = this.user;
		if (user) await this.store.deleteGroup(user, ref).catch(() => {});
		this.groups = this.groups.filter((g) => g.id !== ref);
		return r;
	}

	/** Remove everything this user downloaded on this device, and stop the queue. */
	async removeAll(): Promise<void> {
		const user = this.user;
		if (!user) return;
		await this.cancelAll();
		await this.store.deleteUser(user);
		this.items = [];
		this.saved = [];
		this.groups = [];
	}

	get usedBytes(): number {
		return this.saved.reduce((s, t) => s + t.size, 0);
	}
}
