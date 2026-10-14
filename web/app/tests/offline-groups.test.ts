// Downloads as groups (SPEC §8.1): single tracks, playlists and artists' albums, with
// reference-counted removal; "download on Wi-Fi only" (§8.2) with "Download now"; and the
// storage limit (§8.3), checked before queuing and as each track starts.
import { DownloadManager, LIMIT_MESSAGE, LimitReached, classify, type ManagerDeps } from '$lib/offline/manager.svelte';
import { MemoryBackend, OfflineStore } from '$lib/offline/store';
import { BYTES_PER_SECOND, FALLBACK_SECONDS, MAX_REFS, cleanRefs, estimateBytes, isRef, refsOf } from '$lib/offline/types';
import { body, bytes, fakeServer, key, stored, type FakeTrack } from './helpers/fake-downloads';

const track = (id: number, size = 1000, over: Partial<FakeTrack> = {}): FakeTrack => ({ id, key: key(id), bytes: bytes(size, id), title: `Track ${id}`, ...over });

function setup(tracks: FakeTrack[], over: Partial<ManagerDeps> = {}, store = new OfflineStore(new MemoryBackend())) {
	const server = fakeServer(tracks);
	const m = new DownloadManager({
		store,
		fetch: server.fetch,
		authed: (fn) => fn('tok'),
		sleep: async (_ms, signal) => {
			if (signal.aborted) throw new DOMException('aborted', 'AbortError');
		},
		now: () => 1234,
		...over,
	});
	return { m, server, store };
}

const settle = (m: DownloadManager) =>
	vi.waitFor(() => expect(m.items.every((i) => i.status === 'done' || i.status === 'failed' || i.status === 'paused' || m.isHeld(i))).toBe(true));
const keysOf = (m: DownloadManager) => m.saved.map((t) => t.key).sort();
const refsBy = (m: DownloadManager, id: number) => [...(m.saved.find((t) => t.id === id)?.refs ?? [])].sort();

// --- refs and estimates -------------------------------------------------------------

test('refs are a fixed shape: album or playlist with a positive id, or track', () => {
	for (const ok of ['track', 'album:1', 'playlist:42', 'album:999999999999999']) expect(isRef(ok), ok).toBe(true);
	for (const bad of ['', 'album:', 'album:0', 'album:-1', 'album:01', 'album:1.5', 'artist:3', 'playlist:1/../x', 'track ', 'TRACK', 'album:1000000000000000', '__proto__', 1, null, {}]) {
		expect(isRef(bad), String(bad)).toBe(false);
	}
	expect(cleanRefs(['album:1', 'album:1', 'nope', 7, 'track'])).toEqual(['album:1', 'track']);
	expect(cleanRefs('album:1')).toEqual([]);
	expect(cleanRefs(Array.from({ length: 200 }, (_, i) => `playlist:${i + 1}`))).toHaveLength(MAX_REFS);
	// Tracks stored before refs existed belong to their album (or stand alone).
	expect(refsOf(stored({ albumId: 7 }))).toEqual(['album:7']);
	expect(refsOf(stored({ albumId: null }))).toEqual(['track']);
	expect(refsOf(stored({ refs: ['playlist:3', 'bogus'] }))).toEqual(['playlist:3']);
});

test('size estimates: the file size for originals, else duration at the format’s rate', () => {
	expect(estimateBytes({ filesize: 12_345_678, duration: 10 }, 'original')).toBe(12_345_678);
	expect(estimateBytes({ filesize: '5000', duration: 10 }, 'original')).toBe(5000);
	expect(estimateBytes({ filesize: 12_345_678, duration: 60 }, 'opus')).toBe(60 * BYTES_PER_SECOND.opus);
	expect(estimateBytes({ duration: 60 }, 'mp3')).toBe(60 * 40_000);
	expect(estimateBytes({ duration: 60 }, 'original')).toBe(60 * 176_400);
	// Unknown or hostile values never give NaN, zero or a negative size.
	for (const t of [{}, { duration: -5 }, { duration: 'x' }, { duration: NaN }, { filesize: -1 }, { filesize: '1e9' }, { filesize: 1.5 }, { duration: Infinity }]) {
		const n = estimateBytes(t, 'original');
		expect(Number.isSafeInteger(n) && n > 0, JSON.stringify(t)).toBe(true);
	}
	expect(estimateBytes({}, 'opus')).toBe(FALLBACK_SECONDS * BYTES_PER_SECOND.opus);
	expect(estimateBytes({ duration: 1e12 }, 'opus')).toBe(86_400 * BYTES_PER_SECOND.opus);
});

// --- groups and reference counting ---------------------------------------------------

test('a single track downloads on its own and is marked as such', async () => {
	const { m } = setup([track(1, 100, { album_id: 7 })]);
	await m.setUser('alice');
	expect(await m.request([{ id: 1, albumId: 7 }], { ref: 'track' })).toEqual({ queued: 1 });
	await settle(m);
	expect(refsBy(m, 1)).toEqual(['track']);
});

test('a playlist downloads as a group with its name and order', async () => {
	const { m, store } = setup([track(1, 100, { album_id: 7 }), track(2, 100, { album_id: 8 }), track(3, 100, { album_id: 9 })]);
	await m.setUser('alice');
	const r = await m.request([{ id: 3 }, { id: 1 }, { id: 2 }, { id: 1 }, { id: 'x' }], { ref: 'playlist:5', group: { name: 'Road trip', ids: [3, 1, 2, 1] } });
	expect(r).toEqual({ queued: 3 });
	expect(m.group('playlist:5').count).toBe(3);
	await settle(m);
	expect(m.savedFor('playlist:5').map((t) => t.id).sort()).toEqual([1, 2, 3]);
	expect(m.groups).toEqual([{ user: 'alice', id: 'playlist:5', name: 'Road trip', ids: [3, 1, 2, 1], savedAt: 1234 }]);
	expect(await store.listGroups('alice')).toHaveLength(1);
	// After a reload the group is still there.
	const again = setup([], {}, store);
	await again.m.setUser('alice');
	expect(again.m.groups.map((g) => g.name)).toEqual(['Road trip']);
});

test('removing a playlist keeps tracks an album download still needs, and deletes the rest', async () => {
	const { m, store } = setup([track(1, 100, { album_id: 7 }), track(2, 100, { album_id: 7 }), track(3, 100, { album_id: 9 })]);
	await m.setUser('alice');
	await m.request([{ id: 1, albumId: 7 }, { id: 2, albumId: 7 }], { ref: 'album:7' });
	await settle(m);
	// The playlist shares track 1 with the album: it is not fetched again, just gains the ref.
	await m.request([{ id: 1 }, { id: 3 }], { ref: 'playlist:5', group: { name: 'Mix', ids: [1, 3] } });
	await settle(m);
	expect(refsBy(m, 1)).toEqual(['album:7', 'playlist:5']);
	const r = await m.removePlaylist(5);
	expect(r).toEqual({ removed: [key(3)], kept: 1 });
	expect(keysOf(m)).toEqual([key(1), key(2)]);
	expect(refsBy(m, 1)).toEqual(['album:7']);
	expect(m.groups).toEqual([]);
	expect(await store.listGroups('alice')).toEqual([]);
	// Track 3's audio is gone; track 1's is intact.
	expect(await store.backend.keys('chunks', `alice/${key(3)}`)).toEqual([]);
	expect(await store.getAudio('alice', key(1))).toBeDefined();
	expect((await store.getTrack('alice', key(1)))?.refs).toEqual(['album:7']);
});

test('removing an album keeps tracks a playlist needs; removing that playlist then deletes them', async () => {
	const { m, store } = setup([track(1, 100, { album_id: 7 }), track(2, 100, { album_id: 7 })]);
	await m.setUser('alice');
	await m.request([{ id: 1 }], { ref: 'playlist:5', group: { name: 'Mix', ids: [1] } });
	await m.request([{ id: 1, albumId: 7 }, { id: 2, albumId: 7 }], { ref: 'album:7' });
	await settle(m);
	expect(await m.removeAlbum(7)).toEqual({ removed: [key(2)], kept: 1 });
	expect(keysOf(m)).toEqual([key(1)]);
	expect(m.groups.map((g) => g.id)).toEqual(['playlist:5']);
	expect(await m.removePlaylist(5)).toEqual({ removed: [key(1)], kept: 0 });
	expect(m.saved).toEqual([]);
	// Nothing orphaned: no audio, partials, covers or groups left behind.
	for (const s of ['tracks', 'chunks', 'partials', 'covers', 'groups'] as const) expect(await store.backend.list(s, ''), s).toEqual([]);
});

test('two playlists sharing a track: it goes only with the second', async () => {
	const { m } = setup([track(1), track(2), track(3)]);
	await m.setUser('alice');
	await m.request([{ id: 1 }, { id: 2 }], { ref: 'playlist:1', group: { name: 'A', ids: [1, 2] } });
	await m.request([{ id: 2 }, { id: 3 }], { ref: 'playlist:2', group: { name: 'B', ids: [2, 3] } });
	await settle(m);
	expect((await m.removePlaylist(1)).removed).toEqual([key(1)]);
	expect(keysOf(m)).toEqual([key(2), key(3)]);
	expect((await m.removePlaylist(2)).removed.sort()).toEqual([key(2), key(3)]);
	expect(m.saved).toEqual([]);
});

test('removing a track keeps it for a playlist; tracks stored before groups belong to their album', async () => {
	const { m, store } = setup([]);
	// Stored by an older version: no refs.
	await store.putTrack(stored({ key: key(1), id: 1, albumId: 7 }));
	await store.putChunk('alice', key(1), 0, new Blob([new Uint8Array(4)]));
	await store.putTrack(stored({ key: key(2), id: 2, albumId: 7, refs: ['album:7', 'playlist:4'] }));
	await m.setUser('alice');
	expect(await m.removeTracks([key(1), key(2)])).toEqual({ removed: [key(1)], kept: 1 });
	expect(refsBy(m, 2)).toEqual(['playlist:4']);
	expect(await store.backend.list('chunks', '')).toEqual([]);
});

test('removing a playlist while it is downloading cancels only what nothing else wants', async () => {
	const t1 = track(1, 100);
	const t2 = track(2, 100);
	const { m, server, store } = setup([t1, t2]);
	// Audio never arrives, so both stay in progress.
	server.hooks.push((req) => (req.format ? new Response(body(t1.bytes, 10, undefined, new Promise(() => {})), { headers: { 'content-length': '100' } }) : undefined));
	await m.setUser('alice');
	await m.request([{ id: 1, albumId: 7 }], { ref: 'album:7' });
	await m.request([{ id: 1 }, { id: 2 }], { ref: 'playlist:5', group: { name: 'Mix', ids: [1, 2] } });
	expect(m.item(1)!.refs).toEqual(['album:7', 'playlist:5']);
	await m.removePlaylist(5);
	expect(m.items.map((i) => [i.id, i.refs])).toEqual([[1, ['album:7']]]);
	expect(m.groups).toEqual([]);
	expect(await store.listGroups('alice')).toEqual([]);
	await m.cancelAll();
});

test('the queue keeps refs, "Download now" and the stable key across a reload; bad entries are dropped', async () => {
	const store = new OfflineStore(new MemoryBackend());
	await store.saveQueue('alice', [
		{ id: 1, albumId: 7, choice: 'opus', paused: true, refs: ['playlist:3', 'evil', 'album:7'], cellular: true, estimate: 500, key: key(1) },
		{ id: 2, albumId: null, choice: 'original', paused: true, refs: ['<script>'] as string[], key: '../../x' },
		{ id: -4, albumId: 7, choice: 'original', paused: true },
		{ id: 5, albumId: 'x' as unknown as number, choice: 'nope' as 'mp3', paused: true, cellular: 'yes' as unknown as boolean, estimate: -9 },
	]);
	const { m } = setup([], {}, store);
	await m.setUser('alice');
	expect(m.items.map((i) => ({ id: i.id, refs: i.refs, cellular: i.cellular, estimate: i.estimate, key: i.key, albumId: i.albumId, choice: i.choice }))).toEqual([
		{ id: 1, refs: ['playlist:3', 'album:7'], cellular: true, estimate: 500, key: key(1), albumId: 7, choice: 'opus' },
		{ id: 2, refs: ['track'], cellular: false, estimate: 0, key: '', albumId: null, choice: 'original' },
		{ id: 5, refs: ['track'], cellular: false, estimate: 0, key: '', albumId: null, choice: 'auto' },
	]);
});

test('cancelling a restored download before its details arrive still throws its bytes away', async () => {
	const store = new OfflineStore(new MemoryBackend());
	await store.putPartial({ user: 'alice', key: key(1), format: 'original', total: 10, received: 4, chunks: 1, contentType: 'audio/flac' });
	await store.putChunk('alice', key(1), 0, new Blob([new Uint8Array(4)]));
	await store.saveQueue('alice', [{ id: 1, albumId: 7, choice: 'original', paused: true, key: key(1) }]);
	const { m } = setup([], {}, store);
	await m.setUser('alice');
	await m.cancel(1);
	expect(await store.getPartial('alice', key(1))).toBeUndefined();
	expect(await store.backend.list('chunks', '')).toEqual([]);
});

test('a failed download’s bytes go when the list is cleared', async () => {
	const { m, store } = setup([track(1, 100)], { waitForWifi: () => true });
	await store.putPartial({ user: 'alice', key: key(1), format: 'original', total: 100, received: 4, chunks: 1, contentType: 'audio/flac' });
	await store.putChunk('alice', key(1), 0, new Blob([new Uint8Array(4)]));
	await m.setUser('alice');
	m.enqueue([{ id: 1 }], 'original');
	m.item(1)!.key = key(1);
	m.item(1)!.status = 'failed';
	m.clearFinished();
	await vi.waitFor(async () => expect(await store.getPartial('alice', key(1))).toBeUndefined());
});

test('group ids are checked before they reach storage keys', async () => {
	const store = new OfflineStore(new MemoryBackend());
	for (const id of ['track', '../bob/playlist:1', 'playlist:1/x', '', 'album:0']) {
		await expect(store.putGroup({ user: 'alice', id, name: 'x', ids: [], savedAt: 1 }), id).rejects.toThrow(RangeError);
	}
	// A bad ref in a request falls back to "track" and never names a group.
	const { m } = setup([track(1, 100)], {}, store);
	await m.setUser('alice');
	await m.request([{ id: 1, ref: 'bob/playlist:1' }], { ref: '../x', group: { name: 'evil', ids: [1] } });
	await settle(m);
	expect(refsBy(m, 1)).toEqual(['track']);
	expect(await store.listGroups('alice')).toEqual([]);
});

test('groups are kept per user', async () => {
	const { m, store } = setup([track(1, 100)]);
	await m.setUser('alice');
	await m.request([{ id: 1 }], { ref: 'playlist:5', group: { name: 'Alice’s', ids: [1] } });
	await settle(m);
	await m.setUser('bob');
	expect(m.groups).toEqual([]);
	expect(await store.listGroups('bob')).toEqual([]);
	await m.setUser('alice');
	await m.removeAll();
	expect(await store.listGroups('alice')).toEqual([]);
});

// --- Wi-Fi only ------------------------------------------------------------------------

test('on mobile data with Wi-Fi only, downloads wait; "Download now" lets that batch through', async () => {
	let cellular = true;
	const { m, server } = setup([track(1, 100), track(2, 100), track(3, 100)], { waitForWifi: () => cellular });
	await m.setUser('alice');
	await m.request([{ id: 1 }, { id: 2 }], { ref: 'playlist:5', group: { name: 'Mix', ids: [1, 2] } });
	await m.request([{ id: 3, albumId: 9 }], { ref: 'album:9' });
	expect(m.holding).toBe(true);
	expect(m.items.every((i) => m.isHeld(i))).toBe(true);
	expect(server.reqs).toEqual([]);
	// Only the playlist's downloads go.
	m.downloadNow((i) => i.refs.includes('playlist:5'));
	await vi.waitFor(() => expect(m.item(2)!.status).toBe('done'));
	expect(m.item(1)!.status).toBe('done');
	expect(m.item(3)!).toMatchObject({ status: 'queued', cellular: false });
	expect(m.isHeld(m.item(3)!)).toBe(true);
	// Back on Wi-Fi, the rest carries on by itself.
	cellular = false;
	m.networkChanged();
	await vi.waitFor(() => expect(m.item(3)!.status).toBe('done'));
	expect(m.holding).toBe(false);
});

test('without the Network Information API (Safari, Firefox, desktop) nothing is held', async () => {
	const { m } = setup([track(1, 100)]); // no waitForWifi at all
	await m.setUser('alice');
	await m.request([{ id: 1 }]);
	await settle(m);
	expect(m.item(1)!.status).toBe('done');
	expect(m.holding).toBe(false);
	// A check that throws doesn't hold downloads either.
	const t = setup([track(2, 100)], {
		waitForWifi: () => {
			throw new Error('no API');
		},
	});
	await t.m.setUser('alice');
	await t.m.request([{ id: 2 }]);
	await settle(t.m);
	expect(t.m.item(2)!.status).toBe('done');
});

test('moving onto mobile data stops a download in progress; back on Wi-Fi it resumes by Range', async () => {
	const t = track(1, 4096);
	let cellular = false;
	const { m, server, store } = setup([t], { waitForWifi: () => cellular });
	let release!: () => void;
	const gate = new Promise<void>((r) => (release = r));
	let first = true;
	server.hooks.push((req) => {
		if (!req.format || !first) return undefined;
		first = false;
		// 1 KiB arrives, then the body stalls until the network changes.
		const stream = new ReadableStream<Uint8Array>({
			start(ctl) {
				ctl.enqueue(t.bytes.slice(0, 1024));
			},
			async pull() {
				await gate;
			},
		});
		return new Response(stream, { headers: { 'content-length': String(t.bytes.length) } });
	});
	await m.setUser('alice');
	await m.request([{ id: 1 }], { choice: 'original' });
	await vi.waitFor(() => expect(m.item(1)!.received).toBe(1024));
	cellular = true;
	m.networkChanged();
	expect(m.item(1)!.status).toBe('queued');
	expect(m.isHeld(m.item(1)!)).toBe(true);
	await vi.waitFor(async () => expect((await store.getPartial('alice', key(1)))?.received).toBe(1024));
	release();
	cellular = false;
	m.networkChanged();
	await vi.waitFor(() => expect(m.item(1)!.status).toBe('done'));
	expect(server.reqs.filter((r) => r.format).map((r) => r.range)).toEqual([null, 'bytes=1024-']);
});

test('"Download now" survives a reload', async () => {
	const { m, store } = setup([track(1, 100)], { waitForWifi: () => true });
	await m.setUser('alice');
	await m.request([{ id: 1 }]);
	m.downloadNow();
	await settle(m);
	expect(m.item(1)!.status).toBe('done');
	// Another, paused and allowed: what the page saves for a reload keeps the choice.
	await m.request([{ id: 2 }]);
	m.pause(2);
	m.downloadNow();
	expect((await store.loadQueue('alice')).find((q) => q.id === 2)?.cellular).toBe(true);
});

// --- storage limit and browser quota -----------------------------------------------

test('the storage limit refuses a batch that would not fit, saying how much is needed and free', async () => {
	const MB = 1024 ** 2;
	const { m, server } = setup([track(1, 100)], { capBytes: () => 10 * MB });
	await m.setUser('alice');
	const r = await m.request(
		[
			{ id: 1, filesize: 6 * MB },
			{ id: 2, filesize: 5 * MB },
		],
		{ choice: 'original', ref: 'album:7' },
	);
	expect(r.queued).toBe(0);
	expect(r.refused).toBe(
		'Not enough room under your storage limit: this needs about 11.0 MB, and 10.0 MB of 10.0 MB is free. Remove some downloads, or raise the limit in Settings.',
	);
	expect(m.items).toEqual([]);
	expect(server.reqs).toEqual([]);
});

test('cap arithmetic: exactly full fits, one byte over doesn’t; downloads and the queue count', async () => {
	const { m, store } = setup([], { capBytes: () => 1000, waitForWifi: () => true });
	await store.putTrack(stored({ key: key(9), id: 9, size: 400 }));
	await m.setUser('alice');
	// 400 used; 600 free.
	expect((await m.request([{ id: 1, filesize: 600 }], { choice: 'original' })).queued).toBe(1);
	// The queued one (held for Wi-Fi) is counted: nothing is left.
	const r = await m.request([{ id: 2, filesize: 1 }], { choice: 'original' });
	expect(r.queued).toBe(0);
	expect(r.refused).toContain('0 MB of');
	// Asking again for what is already queued needs no more room.
	expect((await m.request([{ id: 1, filesize: 600 }], { choice: 'original', ref: 'playlist:2' })).refused).toBeUndefined();
	// Tracks already downloaded need no room either: they only gain the ref.
	expect(await m.request([{ id: 9, filesize: 400 }], { ref: 'playlist:2' })).toEqual({ queued: 0 });
	expect(refsBy(m, 9)).toEqual(['album:7', 'playlist:2']);
	// The estimate follows the format: Opus at 20 kB/s for 0.03 s is 600 bytes, no file size.
	await m.cancelAll();
	expect((await m.request([{ id: 3, duration: 0.03, filesize: 99_999 }], { choice: 'opus' })).queued).toBe(1);
});

test('no limit (off) and a limit of nonsense never refuse', async () => {
	for (const cap of [0, -5, NaN, Infinity]) {
		const { m } = setup([], { capBytes: () => cap, waitForWifi: () => true });
		await m.setUser('alice');
		expect((await m.request([{ id: 1, filesize: 1e12 }], { choice: 'original' })).refused, String(cap)).toBeUndefined();
	}
});

test('the browser’s free space is checked too, and an estimate that fails doesn’t block', async () => {
	const MB = 1024 ** 2;
	const { m } = setup([], { storageEstimate: async () => ({ usage: 90 * MB, quota: 100 * MB }), waitForWifi: () => true });
	await m.setUser('alice');
	const r = await m.request([{ id: 1, filesize: 11 * MB }], { choice: 'original' });
	expect(r).toEqual({ queued: 0, refused: 'Not enough space in this browser: this needs about 11.0 MB, and only 10.0 MB is free.' });
	expect((await m.request([{ id: 1, filesize: 10 * MB }], { choice: 'original' })).queued).toBe(1);
	// What is still to arrive for the queue counts against the space.
	expect((await m.request([{ id: 2, filesize: 1 }], { choice: 'original' })).refused).toContain('only 0 MB is free');

	for (const storageEstimate of [
		async () => {
			throw new Error('denied');
		},
		async () => ({}),
		async () => ({ quota: NaN, usage: 5 }),
		async () => ({ quota: 0, usage: 0 }),
	]) {
		const t = setup([], { storageEstimate, waitForWifi: () => true });
		await t.m.setUser('alice');
		expect((await t.m.request([{ id: 1, filesize: 1e9 }], { choice: 'original' })).refused).toBeUndefined();
	}
});

test('a track that turns out bigger than the room left fails with a clear reason, storing nothing', async () => {
	const big = track(1, 5000);
	const { m, store, server } = setup([big], { capBytes: () => 4000 });
	await m.setUser('alice');
	// The estimate (from a wrong size) fits; the server's real size doesn't.
	expect((await m.request([{ id: 1, filesize: 100 }], { choice: 'original' })).queued).toBe(1);
	await settle(m);
	expect(m.item(1)!).toMatchObject({ status: 'failed', message: LIMIT_MESSAGE });
	expect(server.reqs.filter((r) => r.format)).toEqual([]);
	expect(await store.backend.list('chunks', '')).toEqual([]);
});

test('a transcode of unknown size stops once it passes the limit, and its bytes are thrown away', async () => {
	// A very short track, so the estimate fits; the bytes that come are more than it said.
	const t = track(1, 8000, { formats: ['opus'], duration: 0.1 });
	const { m, store, server } = setup([t], { capBytes: () => 6000 });
	// No Content-Length: the size is only known as the bytes arrive.
	server.hooks.push((req) => (req.format ? new Response(body(t.bytes, 1000), { headers: { 'content-type': 'audio/ogg' } }) : undefined));
	await m.setUser('alice');
	m.enqueue([{ id: 1 }], 'opus');
	await settle(m);
	expect(m.item(1)!).toMatchObject({ status: 'failed', message: LIMIT_MESSAGE });
	expect(await store.getPartial('alice', key(1))).toBeUndefined();
	expect(await store.backend.keys('chunks', `alice/${key(1)}`)).toEqual([]);
});

test('LimitReached is permanent with the limit message; a full browser is reported as such', () => {
	expect(classify(new LimitReached())).toEqual({ permanent: true, message: LIMIT_MESSAGE });
	expect(classify(new DOMException('full', 'QuotaExceededError'))).toEqual({ permanent: true, message: 'Not enough storage space on this device.' });
});

test('signed out or offline, a request does nothing', async () => {
	const { m, server } = setup([track(1)]);
	expect(await m.request([{ id: 1 }])).toEqual({ queued: 0 });
	await m.setUser('alice', false);
	expect(await m.request([{ id: 1 }])).toEqual({ queued: 0 });
	expect(server.reqs).toEqual([]);
});
