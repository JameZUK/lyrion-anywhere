// The download manager (SPEC §8.3): a queue that fetches two tracks at a time, shows
// progress per track and per album, pauses, cancels, retries with backoff, and resumes an
// interrupted download from the bytes it already stored (HTTP Range).
import { ApiError, CLIENT_HEADER, bearer } from '$lib/api/http';
import type { OfflineStore } from './store';
import {
	BadInfo,
	TRANSCODE_TYPES,
	audioType,
	downloadUrl,
	infoUrl,
	isFormat,
	parseInfo,
	pickFormat,
	trackId,
	type Format,
	type PartialDownload,
	type QueuedDownload,
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
}

export type ItemStatus = 'queued' | 'active' | 'waiting' | 'paused' | 'done' | 'failed';

export interface DownloadItem {
	id: number;
	albumId: number | null;
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
	/** Set if storage can't be used at all. */
	error = $state('');

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
		this.error = '';
		if (!user) return;
		try {
			const saved = await this.store.listTracks(user);
			if (gen !== this.#gen) return;
			this.saved = saved;
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
				this.#add(trackId(e.id), typeof e.albumId === 'number' ? e.albumId : null, e.choice === 'auto' || isFormat(e.choice) ? e.choice : 'auto', e.paused ? 'paused' : 'queued');
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

	#add(id: number, albumId: number | null, choice: Format | 'auto', status: ItemStatus = 'queued'): DownloadItem {
		const existing = this.item(id);
		if (existing) {
			if (existing.status === 'failed' || existing.status === 'paused' || existing.status === 'done') {
				Object.assign(existing, { status, choice, attempts: 0, waits: 0, message: '' });
			}
			return existing;
		}
		this.items.push({ id, albumId, choice, status, title: '', key: '', format: null, received: 0, total: 0, attempts: 0, waits: 0, message: '' });
		return this.items[this.items.length - 1]!;
	}

	/**
	 * Download tracks (in order). Tracks already on the device are checked by their stable key
	 * and not fetched again. Ids must be LMS track ids; anything else is refused.
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
			.map((i) => ({ id: i.id, albumId: i.albumId, choice: i.choice, paused: i.status === 'paused' }));
		void this.store.saveQueue(user, q).catch(() => {});
	}

	#pump() {
		if (!this.online) return;
		while (this.#active < CONCURRENCY) {
			const next = this.items.find((i) => i.status === 'queued');
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

		// Already here (perhaps under an older track id, before a rescan): just refresh the id.
		const have = await store.getTrack(user, info.key);
		if (have) {
			const t = { ...have, id: info.id, albumId: info.albumId ?? have.albumId };
			if (t.id !== have.id || t.albumId !== have.albumId) await store.putTrack(t);
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
		const take = async (bytes: Uint8Array) => {
			if (signal.aborted) throw abortError();
			buf.push(bytes);
			bufLen += bytes.byteLength;
			item.received += bytes.byteLength;
			if (total && item.received > total) {
				buf = [];
				bufLen = 0;
				await store.discardPartial(user, info.key);
				throw new HttpError(502, 'more data than expected');
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
				if (!(e instanceof HttpError) && gen === this.#gen) await flush().catch(() => {});
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

	pauseAll(albumId?: number) {
		for (const i of this.items) if (albumId === undefined || i.albumId === albumId) this.pause(i.id);
	}

	resumeAll(albumId?: number) {
		for (const i of [...this.items]) if (albumId === undefined || i.albumId === albumId) this.resume(i.id);
	}

	async cancelAll(albumId?: number) {
		await Promise.all(this.items.filter((i) => albumId === undefined || i.albumId === albumId).map((i) => this.cancel(i.id)));
	}

	/** Forget finished and failed entries in the list of downloads. */
	clearFinished() {
		this.items = this.items.filter((i) => i.status !== 'done' && i.status !== 'failed');
		this.#saveQueue();
	}

	album(albumId: number): AlbumProgress {
		const its = this.items.filter((i) => i.albumId === albumId);
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

	/** Remove downloaded tracks from this device. */
	async remove(keys: string[]): Promise<void> {
		const user = this.user;
		if (!user) return;
		for (const k of keys) await this.store.deleteTrack(user, k);
		this.items = this.items.filter((i) => !keys.includes(i.key));
		this.saved = this.saved.filter((t) => !keys.includes(t.key));
		this.#saveQueue();
	}

	async removeAlbum(albumId: number): Promise<void> {
		await this.cancelAll(albumId);
		await this.remove(this.savedAlbum(albumId).map((t) => t.key));
	}

	/** Remove everything this user downloaded on this device, and stop the queue. */
	async removeAll(): Promise<void> {
		const user = this.user;
		if (!user) return;
		await this.cancelAll();
		await this.store.deleteUser(user);
		this.items = [];
		this.saved = [];
	}

	get usedBytes(): number {
		return this.saved.reduce((s, t) => s + t.size, 0);
	}
}
