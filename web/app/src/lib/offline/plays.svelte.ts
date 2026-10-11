// Offline play sync-back (SPEC D6, docs/PROTOCOL.md "Offline plays").
//
// The offline player reports each play it finishes or leaves (track, start time, seconds
// heard). Each one is queued in IndexedDB, per user, next to the downloads, and sent to
// POST /v1/plays in batches whenever this user is signed in and online. A play stays queued
// until the server has answered for it: recorded, duplicate (it already had it) or refused
// (it never will count: too short, not in the library, too old…). "Retry" answers and
// failures keep it, and sending is tried again with backoff.
//
// The server decides what counts (half the track or four minutes); every play is sent, so
// the rule lives in one place. The queue is capped; beyond it the oldest plays go.
import { ApiError, CLIENT_HEADER, bearer } from '$lib/api/http';
import { HttpError, retryAfter, type Authed } from './manager.svelte';
import { PLAY_ID, type OfflineStore } from './store';
import type { QueuedPlay, StoredTrack } from './types';

/** Most plays in one upload (the server's limit). */
export const BATCH = 200;
/** Most plays kept waiting per user; beyond this the oldest are dropped. */
export const MAX_QUEUED = 5000;
/** Plays shorter than this aren't worth keeping (a track skipped straight away). */
export const MIN_SECONDS = 1;
/** Longest a single play can be (the server's limit). */
export const MAX_SECONDS = 86_400;
/** Plays older than this are refused by the server: no point keeping them. */
export const MAX_AGE_S = 90 * 86_400;

/** Backoff between failed attempts: 5 s, 10 s, 20 s… up to 10 minutes. */
export const playBackoff = (failures: number) => Math.min(600_000, 5000 * 2 ** Math.max(0, failures - 1));

export interface PlayResult {
	key: string | null;
	result: 'recorded' | 'duplicate' | 'refused' | 'retry';
	reason?: string;
}

/** Check the server's answer: one result per play sent, in order. */
export function parseResults(raw: unknown, sent: number): PlayResult[] {
	const r = raw && typeof raw === 'object' && !Array.isArray(raw) ? (raw as Record<string, unknown>).results : undefined;
	if (!Array.isArray(r) || r.length !== sent) throw new Error('unexpected answer');
	return r.map((x) => {
		const o = x && typeof x === 'object' ? (x as Record<string, unknown>) : {};
		const result = o.result;
		if (result !== 'recorded' && result !== 'duplicate' && result !== 'refused' && result !== 'retry') throw new Error('unexpected answer');
		return { key: typeof o.key === 'string' ? o.key : null, result, ...(typeof o.reason === 'string' ? { reason: o.reason.slice(0, 40) } : {}) };
	});
}

/** The upload body for queued plays. */
export function body(plays: QueuedPlay[]) {
	return plays.map((p) => ({ key: p.id, track_id: p.trackId, played_at: p.playedAt, seconds: Math.round(p.seconds * 10) / 10, track_key: p.trackKey }));
}

export interface PlaySyncDeps {
	store: OfflineStore;
	authed: Authed;
	fetch?: typeof fetch;
	now?: () => number;
	uuid?: () => string;
	/** Timers for retries (tests replace them). */
	setTimer?: (fn: () => void, ms: number) => unknown;
	clearTimer?: (t: unknown) => void;
	/** Most plays kept waiting per user (default [`MAX_QUEUED`]). */
	maxQueued?: number;
}

function defaultUuid(): string {
	if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') return crypto.randomUUID();
	const b = new Uint8Array(16);
	crypto.getRandomValues(b);
	b[6] = (b[6]! & 0x0f) | 0x40;
	b[8] = (b[8]! & 0x3f) | 0x80;
	const h = [...b].map((x) => x.toString(16).padStart(2, '0')).join('');
	return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}

export class PlaySync {
	/** Plays waiting to be sent, for the user shown. */
	pending = $state(0);
	user = $state<string | null>(null);
	online = $state(false);

	#gen = 0;
	#running = false;
	#again = false;
	#failures = 0;
	#timer: unknown = null;
	#fetch: typeof fetch;
	#now: () => number;
	#uuid: () => string;
	#set: (fn: () => void, ms: number) => unknown;
	#clear: (t: unknown) => void;

	constructor(private deps: PlaySyncDeps) {
		this.#fetch = deps.fetch ?? ((...a) => fetch(...a));
		this.#now = deps.now ?? (() => Date.now());
		this.#uuid = deps.uuid ?? defaultUuid;
		this.#set = deps.setTimer ?? ((fn, ms) => setTimeout(fn, ms));
		this.#clear = deps.clearTimer ?? ((t) => clearTimeout(t as ReturnType<typeof setTimeout>));
	}

	/** Whose plays to show and send (null when signed out). Sending needs `online`. */
	async setUser(user: string | null, online = true): Promise<void> {
		const changed = user !== this.user;
		this.user = user;
		this.online = online && !!user;
		if (changed) {
			this.#gen++;
			this.#failures = 0;
			this.#cancelTimer();
			this.pending = 0;
		}
		if (user) await this.#count(user);
		if (this.online) this.kick();
	}

	async #count(user: string) {
		const n = (await this.deps.store.playIds(user).catch(() => [])).length;
		if (user === this.user) this.pending = n;
	}

	/**
	 * Note a play by `user` of a downloaded track: started at `startedMs`, `seconds` heard.
	 * Plays too short to matter are ignored.
	 */
	async record(user: string, t: Pick<StoredTrack, 'id' | 'key'>, startedMs: number, seconds: number): Promise<void> {
		if (!user || !Number.isFinite(seconds) || seconds < MIN_SECONDS) return;
		if (!Number.isSafeInteger(t.id) || t.id <= 0 || !/^[0-9a-f]{64}$/.test(t.key)) return;
		const playedAt = Math.floor(startedMs / 1000);
		if (!Number.isSafeInteger(playedAt) || playedAt <= 0 || playedAt > 99_999_999_999) return;
		const id = `${String(playedAt).padStart(11, '0')}-${this.#uuid().toLowerCase()}`;
		if (!PLAY_ID.test(id)) return;
		const play: QueuedPlay = { user, id, trackId: t.id, trackKey: t.key, playedAt, seconds: Math.min(seconds, MAX_SECONDS) };
		const store = this.deps.store;
		try {
			await store.addPlay(play);
			// Keep the queue bounded: the oldest go first.
			const ids = await store.playIds(user);
			for (const old of ids.slice(0, Math.max(0, ids.length - (this.deps.maxQueued ?? MAX_QUEUED)))) await store.deletePlay(user, old);
		} catch {
			return; // storage unavailable: the play is lost, playback carries on
		}
		if (user === this.user) await this.#count(user);
		this.kick();
	}

	/** Send now, if there is anything to send and we may (no-op while already sending). */
	kick() {
		if (!this.online || !this.user) return;
		if (this.#running) {
			this.#again = true;
			return;
		}
		this.#cancelTimer();
		void this.#run();
	}

	#cancelTimer() {
		if (this.#timer !== null) this.#clear(this.#timer);
		this.#timer = null;
	}

	#later(ms: number, gen: number) {
		this.#cancelTimer();
		this.#timer = this.#set(() => {
			this.#timer = null;
			if (gen === this.#gen) this.kick();
		}, ms);
	}

	/** Send everything queued, a batch at a time, until done or the server says wait. */
	async #run(): Promise<void> {
		const user = this.user!;
		const gen = this.#gen;
		this.#running = true;
		try {
			do {
				this.#again = false;
				const wait = await this.#sendAll(user, gen);
				if (gen !== this.#gen) return;
				if (wait !== null) {
					this.#later(wait, gen);
					return;
				}
			} while (this.#again && this.online && gen === this.#gen);
		} finally {
			this.#running = false;
			if (gen === this.#gen) await this.#count(user);
		}
	}

	/** null when everything was sent (or nothing could be); else how long to wait. */
	async #sendAll(user: string, gen: number): Promise<number | null> {
		const store = this.deps.store;
		let size = BATCH;
		const skip = new Set<string>();
		for (;;) {
			if (gen !== this.#gen || !this.online) return null;
			const minAt = Math.floor(this.#now() / 1000) - MAX_AGE_S;
			let all: QueuedPlay[];
			try {
				all = await store.listPlays(user);
			} catch {
				return playBackoff(++this.#failures);
			}
			// Plays the server would refuse as too old aren't sent; malformed ones are dropped.
			const batch: QueuedPlay[] = [];
			for (const p of all) {
				if (skip.has(p.id)) continue;
				if (!p || !PLAY_ID.test(p.id) || p.playedAt < minAt) {
					await store.deletePlay(user, p.id).catch(() => {});
					continue;
				}
				batch.push(p);
				if (batch.length >= size) break;
			}
			if (!batch.length) {
				this.#failures = 0;
				return skip.size ? 60_000 : null;
			}
			let results: PlayResult[];
			try {
				results = await this.deps.authed((token) => this.#post(batch, token));
			} catch (e) {
				if (e instanceof ApiError) {
					const s = e.status;
					if (s === 401) return null; // signed out: nothing to do until sign-in
					if ((s === 400 || s === 413) && batch.length > 1) {
						size = Math.max(1, Math.floor(batch.length / 2));
						continue;
					}
					if (s === 400 || s === 413) {
						// The server will never take this one: don't let it block the rest.
						await store.deletePlay(user, batch[0]!.id).catch(() => {});
						continue;
					}
					const ra = e instanceof HttpError ? e.retryAfterMs : undefined;
					return ra ?? playBackoff(++this.#failures);
				}
				return playBackoff(++this.#failures);
			}
			if (gen !== this.#gen) return null;
			this.#failures = 0;
			let retry = false;
			for (const [i, r] of results.entries()) {
				const p = batch[i]!;
				if (r.result === 'retry') {
					retry = true;
					skip.add(p.id);
				} else await store.deletePlay(user, p.id).catch(() => {});
			}
			if (user === this.user) await this.#count(user);
			// A "retry" answer means the server can't take more just now: wait a while.
			if (retry) return 60_000;
		}
	}

	async #post(batch: QueuedPlay[], token: string): Promise<PlayResult[]> {
		const resp = await this.#fetch('/v1/plays', {
			method: 'POST',
			headers: { ...CLIENT_HEADER, ...bearer(token), 'Content-Type': 'application/json' },
			credentials: 'same-origin',
			cache: 'no-store',
			body: JSON.stringify(body(batch)),
		});
		if (!resp.ok) throw new HttpError(resp.status, `HTTP ${resp.status}`, resp.status === 429 || resp.status === 503 ? retryAfter(resp.headers.get('retry-after')) : undefined);
		let raw: unknown;
		try {
			raw = await resp.json();
		} catch {
			throw new Error('unexpected answer');
		}
		return parseResults(raw, batch.length);
	}
}
