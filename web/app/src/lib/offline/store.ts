// Where downloads are kept on this device.
//
// Storage choice: IndexedDB, for both the audio and its details.
// - It is the one store that holds Blobs reliably on every browser we target: iOS Safari 17+,
//   Android Chrome and Firefox, and desktop. OPFS (the spec's first idea) can't be written
//   from the page on Safari 17 (createWritable came later; only worker-side sync handles
//   exist), so it would need a worker per write and still fail on older iOS.
// - Cache Storage would be wiped by our own service worker, which deletes every cache but
//   the current shell on each update; it also keys by URL, and our download URLs carry
//   track ids that change on a rescan.
// - Audio is stored in pieces (about 1 MiB each) as it arrives, so an interrupted download
//   keeps what it has and resumes with an HTTP Range request, even after a reload.
//
// Plays made offline wait in 'plays' until the server has them (see ./plays.svelte).
//
// Everything is partitioned by user: each key starts with the (encoded) username, so a
// different user signing in on the same browser never sees another user's downloads.
// Tokens are never stored.
import type { PartialDownload, QueuedDownload, QueuedPlay, StoredTrack } from './types';

export type StoreName = 'tracks' | 'chunks' | 'partials' | 'covers' | 'queues' | 'plays';
const STORES: StoreName[] = ['tracks', 'chunks', 'partials', 'covers', 'queues', 'plays'];

/** The handful of operations the store needs: real (IndexedDB) or in memory (tests). */
export interface Backend {
	get<T>(store: StoreName, key: string): Promise<T | undefined>;
	put(store: StoreName, key: string, value: unknown): Promise<void>;
	del(store: StoreName, key: string): Promise<void>;
	/** Values whose keys start with `prefix`, in key order. */
	list<T>(store: StoreName, prefix: string): Promise<T[]>;
	/** Keys that start with `prefix`, in order. */
	keys(store: StoreName, prefix: string): Promise<string[]>;
	delPrefix(store: StoreName, prefix: string): Promise<void>;
}

export class MemoryBackend implements Backend {
	readonly data = new Map<StoreName, Map<string, unknown>>(STORES.map((s) => [s, new Map()]));
	#copy = (v: unknown) => (v && typeof v === 'object' && Object.getPrototypeOf(v) === Object.prototype ? { ...v } : v);
	async get<T>(s: StoreName, k: string) {
		return this.#copy(this.data.get(s)!.get(k)) as T | undefined;
	}
	async put(s: StoreName, k: string, v: unknown) {
		this.data.get(s)!.set(k, this.#copy(v));
	}
	async del(s: StoreName, k: string) {
		this.data.get(s)!.delete(k);
	}
	async list<T>(s: StoreName, prefix: string) {
		return [...this.data.get(s)!.entries()]
			.filter(([k]) => k.startsWith(prefix))
			.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
			.map(([, v]) => this.#copy(v) as T);
	}
	async keys(s: StoreName, prefix: string) {
		return [...this.data.get(s)!.keys()].filter((k) => k.startsWith(prefix)).sort();
	}
	async delPrefix(s: StoreName, prefix: string) {
		for (const k of [...this.data.get(s)!.keys()]) if (k.startsWith(prefix)) this.data.get(s)!.delete(k);
	}
}

/** IndexedDB, one object store per kind, string keys. Thin on purpose: the logic is above. */
export class IdbBackend implements Backend {
	#db: Promise<IDBDatabase> | null = null;
	constructor(private name = 'anywhere-offline') {}

	#open(): Promise<IDBDatabase> {
		this.#db ??= new Promise<IDBDatabase>((resolve, reject) => {
			if (typeof indexedDB === 'undefined') return reject(new Error('This browser can’t store downloads.'));
			// Version 2 added 'plays'; the upgrade only creates the stores that are missing.
			const req = indexedDB.open(this.name, 2);
			req.onupgradeneeded = () => {
				for (const s of STORES) if (!req.result.objectStoreNames.contains(s)) req.result.createObjectStore(s);
			};
			req.onsuccess = () => {
				// A newer version of the app in another tab needs to upgrade: let it.
				req.result.onversionchange = () => {
					req.result.close();
					this.#db = null;
				};
				resolve(req.result);
			};
			req.onerror = () => reject(req.error ?? new Error('storage unavailable'));
			req.onblocked = () => reject(new Error('storage is busy in another tab'));
		});
		this.#db.catch(() => (this.#db = null));
		return this.#db;
	}

	async #run<T>(s: StoreName, mode: IDBTransactionMode, op: (os: IDBObjectStore) => IDBRequest | void): Promise<T> {
		const db = await this.#open();
		return new Promise<T>((resolve, reject) => {
			const tx = db.transaction(s, mode);
			const req = op(tx.objectStore(s));
			tx.oncomplete = () => resolve((req ? req.result : undefined) as T);
			tx.onerror = () => reject(tx.error ?? new Error('storage error'));
			tx.onabort = () => reject(tx.error ?? new Error('storage error'));
		});
	}

	#range = (prefix: string) => IDBKeyRange.bound(prefix, `${prefix}￿`);

	get<T>(s: StoreName, k: string) {
		return this.#run<T | undefined>(s, 'readonly', (os) => os.get(k));
	}
	put(s: StoreName, k: string, v: unknown) {
		return this.#run<void>(s, 'readwrite', (os) => void os.put(v, k));
	}
	del(s: StoreName, k: string) {
		return this.#run<void>(s, 'readwrite', (os) => void os.delete(k));
	}
	list<T>(s: StoreName, prefix: string) {
		return this.#run<T[]>(s, 'readonly', (os) => os.getAll(this.#range(prefix)));
	}
	keys(s: StoreName, prefix: string) {
		return this.#run<IDBValidKey[]>(s, 'readonly', (os) => os.getAllKeys(this.#range(prefix))).then((ks) => ks.map(String));
	}
	delPrefix(s: StoreName, prefix: string) {
		return this.#run<void>(s, 'readwrite', (os) => void os.delete(this.#range(prefix)));
	}
}

/** The key prefix for one user's data. Encoded, so no username can reach into another's. */
export const userPrefix = (user: string) => {
	if (typeof user !== 'string' || !user) throw new Error('no user');
	return `${encodeURIComponent(user)}/`;
};
const trackKey = (user: string, key: string) => {
	if (!/^[0-9a-f]{64}$/.test(key)) throw new RangeError('bad track key');
	return `${userPrefix(user)}${key}`;
};
const coverKey = (user: string, albumId: number) => {
	if (!Number.isSafeInteger(albumId) || albumId <= 0) throw new RangeError('bad album id');
	return `${userPrefix(user)}${albumId}`;
};
const seqKey = (n: number) => String(n).padStart(8, '0');
/** A queued play's id: its start time (so plays list oldest first) and a random part. */
export const PLAY_ID = /^\d{11}-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const playKey = (user: string, id: string) => {
	if (typeof id !== 'string' || !PLAY_ID.test(id)) throw new RangeError('bad play id');
	return `${userPrefix(user)}${id}`;
};

export class OfflineStore {
	constructor(readonly backend: Backend) {}

	async listTracks(user: string): Promise<StoredTrack[]> {
		return this.backend.list<StoredTrack>('tracks', userPrefix(user));
	}
	async getTrack(user: string, key: string): Promise<StoredTrack | undefined> {
		return this.backend.get<StoredTrack>('tracks', trackKey(user, key));
	}
	async putTrack(t: StoredTrack): Promise<void> {
		await this.backend.put('tracks', trackKey(t.user, t.key), { ...t });
	}

	async putChunk(user: string, key: string, seq: number, blob: Blob): Promise<void> {
		return this.backend.put('chunks', `${trackKey(user, key)}/${seqKey(seq)}`, blob);
	}
	/** A downloaded track's audio, in one Blob of its content type. */
	async getAudio(user: string, key: string): Promise<Blob | undefined> {
		const t = await this.getTrack(user, key);
		if (!t) return undefined;
		const parts = await this.backend.list<Blob>('chunks', `${trackKey(user, key)}/`);
		if (parts.length !== t.chunks) return undefined; // incomplete: treat as missing
		return new Blob(parts, { type: t.contentType });
	}

	async getPartial(user: string, key: string): Promise<PartialDownload | undefined> {
		return this.backend.get<PartialDownload>('partials', trackKey(user, key));
	}
	async putPartial(p: PartialDownload): Promise<void> {
		return this.backend.put('partials', trackKey(p.user, p.key), { ...p });
	}
	/** Forget a partial download's record (its pieces stay: they become the track's). */
	async finishPartial(user: string, key: string): Promise<void> {
		return this.backend.del('partials', trackKey(user, key));
	}
	/** Throw away a partial download and what it had stored. */
	async discardPartial(user: string, key: string): Promise<void> {
		await this.backend.del('partials', trackKey(user, key));
		await this.backend.delPrefix('chunks', `${trackKey(user, key)}/`);
	}

	async getCover(user: string, albumId: number): Promise<Blob | undefined> {
		return this.backend.get<Blob>('covers', coverKey(user, albumId));
	}
	async putCover(user: string, albumId: number, blob: Blob): Promise<void> {
		return this.backend.put('covers', coverKey(user, albumId), blob);
	}

	async loadQueue(user: string): Promise<QueuedDownload[]> {
		const q = await this.backend.get<QueuedDownload[]>('queues', userPrefix(user));
		return Array.isArray(q) ? q : [];
	}
	async saveQueue(user: string, q: QueuedDownload[]): Promise<void> {
		return q.length ? this.backend.put('queues', userPrefix(user), q.map((e) => ({ ...e }))) : this.backend.del('queues', userPrefix(user));
	}

	/** Queue a play made offline until the server has it. */
	async addPlay(p: QueuedPlay): Promise<void> {
		return this.backend.put('plays', playKey(p.user, p.id), { ...p });
	}
	/** This user's queued plays, oldest first. */
	async listPlays(user: string): Promise<QueuedPlay[]> {
		return this.backend.list<QueuedPlay>('plays', userPrefix(user));
	}
	/** The ids of this user's queued plays, oldest first. */
	async playIds(user: string): Promise<string[]> {
		const p = userPrefix(user);
		return (await this.backend.keys('plays', p)).map((k) => k.slice(p.length));
	}
	async deletePlay(user: string, id: string): Promise<void> {
		return this.backend.del('plays', playKey(user, id));
	}

	/** Remove one downloaded track (and any partial download of it). */
	async deleteTrack(user: string, key: string): Promise<void> {
		const k = trackKey(user, key);
		await this.backend.del('tracks', k);
		await this.backend.del('partials', k);
		await this.backend.delPrefix('chunks', `${k}/`);
		// Drop covers once nothing of their album is left.
		const albums = new Set((await this.listTracks(user)).map((t) => String(t.albumId)));
		const p = userPrefix(user);
		for (const c of await this.backend.keys('covers', p)) if (!albums.has(c.slice(p.length))) await this.backend.del('covers', c);
	}

	/** Remove everything this user downloaded on this device. Plays waiting to be sent are
	 *  kept: they are the user's listening history, not downloads, and go on next sign-in. */
	async deleteUser(user: string): Promise<void> {
		const p = userPrefix(user);
		for (const s of STORES) {
			if (s === 'plays') continue;
			if (s === 'queues') await this.backend.del(s, p);
			else await this.backend.delPrefix(s, p);
		}
	}
}
