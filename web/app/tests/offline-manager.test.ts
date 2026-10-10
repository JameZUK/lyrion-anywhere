// The download manager against a fake gateway: queue and concurrency, progress, pause and
// cancel, Range resume, 429 Retry-After, token renewal on 401, refusals, and per-user data.
import { DownloadManager, CHUNK_BYTES, MAX_ATTEMPTS, type ManagerDeps } from '$lib/offline/manager.svelte';
import { MemoryBackend, OfflineStore } from '$lib/offline/store';
import { SessionManager, type AuthApi } from '$lib/api/session';
import { ApiError } from '$lib/api/http';
import { body, bytes, fakeServer, key, sameBytes, type FakeTrack } from './helpers/fake-downloads';

const track = (id: number, size = 1000, over: Partial<FakeTrack> = {}): FakeTrack => ({ id, key: key(id), bytes: bytes(size, id), title: `Track ${id}`, ...over });

function setup(tracks: FakeTrack[], over: Partial<ManagerDeps> = {}) {
	const server = fakeServer(tracks);
	const store = new OfflineStore(new MemoryBackend());
	const sleeps: number[] = [];
	const tokens: string[] = [];
	const m = new DownloadManager({
		store,
		fetch: server.fetch,
		authed: (fn) => {
			tokens.push('tok');
			return fn('tok');
		},
		sleep: async (ms, signal) => {
			sleeps.push(ms);
			if (signal.aborted) throw new DOMException('aborted', 'AbortError');
		},
		cover: async () => new Blob(['jpeg'], { type: 'image/jpeg' }),
		now: () => 1234,
		...over,
	});
	return { m, server, store, sleeps };
}

const settle = (m: DownloadManager) => vi.waitFor(() => expect(m.items.every((i) => i.status === 'done' || i.status === 'failed' || i.status === 'paused')).toBe(true));

test('downloads a track with its details and cover, keyed by the stable key', async () => {
	const t = track(5, 3 * CHUNK_BYTES + 17, { album_id: 9 });
	const { m, server, store } = setup([t]);
	await m.setUser('alice');
	expect(m.enqueue([{ id: 5, albumId: 9 }], 'original')).toBe(1);
	await settle(m);
	const it = m.item(5)!;
	expect(it.status).toBe('done');
	expect(it.received).toBe(t.bytes.length);
	expect(it.total).toBe(t.bytes.length);
	const saved = await store.getTrack('alice', key(5));
	expect(saved).toMatchObject({ id: 5, albumId: 9, format: 'original', codec: 'flc', ext: 'flac', contentType: 'audio/flac', size: t.bytes.length, downloadedAt: 1234 });
	expect(await sameBytes(await store.getAudio('alice', key(5)), t.bytes)).toBe(true);
	expect((await store.getCover('alice', 9))?.type).toBe('image/jpeg');
	expect(m.saved.map((s) => s.key)).toEqual([key(5)]);
	expect(await store.getPartial('alice', key(5))).toBeUndefined();
	// Every request carried the bearer token and the client header; nothing else in the URL.
	for (const r of server.reqs) {
		expect(r.auth).toBe('Bearer tok');
		expect(r.client).toBe('1');
	}
	expect(server.reqs.map((r) => r.url)).toEqual(['/v1/download/track/5/info', '/v1/download/track/5?format=original']);
});

test('at most two downloads run at once, in order', async () => {
	const tracks = [1, 2, 3, 4, 5].map((i) => track(i));
	const { m, server } = setup(tracks);
	let inFlight = 0;
	let most = 0;
	const gates: (() => void)[] = [];
	server.hooks.push((req) => {
		if (req.format === null) return undefined;
		const t = tracks.find((x) => req.path.endsWith(`/${x.id}`))!;
		inFlight++;
		most = Math.max(most, inFlight);
		const gate = new Promise<void>((r) => gates.push(r));
		const done = gate.then(() => inFlight--);
		void done;
		return new Response(body(t.bytes, 1000, undefined, gate), { headers: { 'content-length': String(t.bytes.length) } });
	});
	await m.setUser('alice');
	m.enqueue(tracks.map((t) => ({ id: t.id, albumId: 7 })));
	for (let i = 0; i < 5; i++) {
		await vi.waitFor(() => expect(gates.length).toBeGreaterThan(i));
		expect(m.items.filter((x) => x.status === 'active').length).toBeLessThanOrEqual(2);
		expect(gates.length).toBeLessThanOrEqual(i + 2);
		gates[i]!();
	}
	await settle(m);
	expect(most).toBe(2);
	expect(m.items.map((i) => i.status)).toEqual(['done', 'done', 'done', 'done', 'done']);
	const audio = server.reqs.filter((r) => r.format).map((r) => r.path);
	expect(audio).toEqual(['/v1/download/track/1', '/v1/download/track/2', '/v1/download/track/3', '/v1/download/track/4', '/v1/download/track/5']);
});

test('album progress counts tracks and bytes', async () => {
	const { m } = setup([track(1), track(2)]);
	await m.setUser('alice');
	expect(m.album(7)).toMatchObject({ count: 0, fraction: 0, busy: false });
	m.enqueue([{ id: 1, albumId: 7 }, { id: 2, albumId: 7 }]);
	expect(m.album(7).busy).toBe(true);
	await settle(m);
	expect(m.album(7)).toMatchObject({ count: 2, done: 2, failed: 0, busy: false, fraction: 1 });
	m.items[1]!.status = 'active';
	m.items[1]!.received = 250;
	m.items[1]!.total = 1000;
	expect(m.album(7).fraction).toBeCloseTo(0.625);
});

test('an interrupted download resumes with Range from what was stored', async () => {
	const t = track(3, 2 * CHUNK_BYTES + 500);
	const { m, server, store } = setup([t]);
	let first = true;
	server.hooks.push((req) => {
		if (req.format && first) {
			first = false;
			// The connection drops part-way through the second piece.
			return new Response(body(t.bytes, 64 * 1024, CHUNK_BYTES + 4096), { headers: { 'content-length': String(t.bytes.length) } });
		}
		return undefined;
	});
	await m.setUser('alice');
	m.enqueue([{ id: 3 }], 'original');
	await settle(m);
	const audio = server.reqs.filter((r) => r.format);
	expect(audio.map((r) => r.range)).toEqual([null, `bytes=${CHUNK_BYTES + 4096}-`]);
	expect(m.item(3)!.status).toBe('done');
	expect(await sameBytes(await store.getAudio('alice', key(3)), t.bytes)).toBe(true);
});

test('a download interrupted by a reload carries on from its stored bytes', async () => {
	const t = track(4, 2 * CHUNK_BYTES);
	const first = setup([t]);
	// The body stalls after the first piece; then the "page" goes away.
	let stall!: () => void;
	first.server.hooks.push((req) => {
		if (!req.format) return undefined;
		const gate = new Promise<void>((r) => (stall = r));
		let sent = false;
		const stream = new ReadableStream<Uint8Array>({
			async pull(ctl) {
				if (!sent) {
					sent = true;
					ctl.enqueue(t.bytes.slice(0, CHUNK_BYTES));
					return;
				}
				await gate; // never released
			},
		});
		return new Response(stream, { headers: { 'content-length': String(t.bytes.length) } });
	});
	await first.m.setUser('alice');
	first.m.enqueue([{ id: 4, albumId: 7 }], 'original');
	await vi.waitFor(async () => expect((await first.store.getPartial('alice', key(4)))?.received).toBe(CHUNK_BYTES));
	void stall;
	await first.m.setUser(null); // the old page stops

	const server = fakeServer([t]);
	const m = new DownloadManager({ store: first.store, fetch: server.fetch, authed: (fn) => fn('tok'), sleep: async () => {} });
	await m.setUser('alice');
	expect(m.items.map((i) => i.id)).toEqual([4]);
	await settle(m);
	expect(server.reqs.filter((r) => r.format).map((r) => r.range)).toEqual([`bytes=${CHUNK_BYTES}-`]);
	expect(await sameBytes(await first.store.getAudio('alice', key(4)), t.bytes)).toBe(true);
});

test('a server that ignores Range starts over cleanly', async () => {
	const t = track(6, CHUNK_BYTES + 10);
	const { m, server, store } = setup([t]);
	await store.putPartial({ user: 'alice', key: key(6), format: 'original', total: t.bytes.length, received: 5, chunks: 1, contentType: 'audio/flac' });
	await store.putChunk('alice', key(6), 0, new Blob([new Uint8Array([9, 9, 9, 9, 9])]));
	server.hooks.push((req) => (req.format ? new Response(body(t.bytes), { status: 200, headers: { 'content-length': String(t.bytes.length) } }) : undefined));
	await m.setUser('alice');
	m.enqueue([{ id: 6 }], 'original');
	await settle(m);
	expect(server.reqs.find((r) => r.format)?.range).toBe('bytes=5-');
	expect(await sameBytes(await store.getAudio('alice', key(6)), t.bytes)).toBe(true);
});

test('a 206 for the wrong range is not stitched on', async () => {
	const t = track(8, 100);
	const { m, server, store } = setup([t]);
	await store.putPartial({ user: 'alice', key: key(8), format: 'original', total: 100, received: 50, chunks: 1, contentType: 'audio/flac' });
	await store.putChunk('alice', key(8), 0, new Blob([t.bytes.slice(0, 50)]));
	let n = 0;
	server.hooks.push((req) => {
		if (req.format && n++ === 0) return new Response(body(t.bytes.slice(10)), { status: 206, headers: { 'content-range': 'bytes 10-99/100', 'content-length': '90' } });
		return undefined;
	});
	await m.setUser('alice');
	m.enqueue([{ id: 8 }], 'original');
	await settle(m);
	expect(server.reqs.filter((r) => r.format).map((r) => r.range)).toEqual(['bytes=50-', null]);
	expect(await sameBytes(await store.getAudio('alice', key(8)), t.bytes)).toBe(true);
});

test('more bytes than promised are thrown away, not stored', async () => {
	const t = track(9, 100);
	const { m, server, store } = setup([t]);
	server.hooks.push((req) => (req.format ? new Response(body(bytes(5000)), { headers: { 'content-length': '100' } }) : undefined));
	await m.setUser('alice');
	m.enqueue([{ id: 9 }], 'original');
	await settle(m);
	expect(m.item(9)!.status).toBe('failed');
	expect(await store.getTrack('alice', key(9))).toBeUndefined();
	expect(await store.backend.list('chunks', '')).toHaveLength(0);
});

test('429 waits for Retry-After, without using up attempts', async () => {
	const { m, server, sleeps } = setup([track(1)]);
	let n = 0;
	server.hooks.push((req) => (req.format && n++ < 3 ? new Response('', { status: 429, headers: { 'retry-after': '7' } }) : undefined));
	await m.setUser('alice');
	m.enqueue([{ id: 1 }]);
	await settle(m);
	expect(sleeps).toEqual([7000, 7000, 7000]);
	expect(m.item(1)!).toMatchObject({ status: 'done', attempts: 0, waits: 3 });
});

test('server errors back off and retry, then give up', async () => {
	const { m, server, sleeps } = setup([track(1), track(2)]);
	let n = 0;
	server.hooks.push((req) => (req.path.includes('/1/') && n++ < 2 ? new Response('', { status: 503 }) : undefined));
	server.hooks.push((req) => (req.path.includes('/2/') ? new Response('', { status: 500 }) : undefined));
	await m.setUser('alice');
	m.enqueue([{ id: 1 }]);
	await settle(m);
	expect(m.item(1)!.status).toBe('done');
	expect(sleeps).toEqual([2000, 4000]);
	m.enqueue([{ id: 2 }]);
	await settle(m);
	expect(m.item(2)!).toMatchObject({ status: 'failed', attempts: MAX_ATTEMPTS, message: 'Download failed. Try again later.' });
});

test('a 401 renews the access token once and carries on', async () => {
	let token = 'old';
	const auth: AuthApi = {
		login: async () => ({ access_token: 'old', expires_in: 900, user: { username: 'alice', role: 'listener' }, device: { id: 1, name: 'x' } }),
		refresh: async () => {
			token = 'new';
			return { access_token: 'new', expires_in: 900, user: { username: 'alice', role: 'listener' }, device: { id: 1, name: 'x' } };
		},
		logout: async () => {},
	};
	const session = new SessionManager(auth, undefined);
	await session.signIn('alice', 'pw');
	const t = track(1);
	const server = fakeServer([t]);
	server.hooks.push((req) => (req.auth !== `Bearer ${token}` || token === 'old' ? new Response('', { status: 401 }) : undefined));
	const m = new DownloadManager({ store: new OfflineStore(new MemoryBackend()), fetch: server.fetch, authed: (fn) => session.authed(fn), sleep: async () => {} });
	await m.setUser('alice');
	m.enqueue([{ id: 1 }]);
	await settle(m);
	expect(m.item(1)!.status).toBe('done');
	expect(server.reqs.map((r) => r.auth)).toEqual(['Bearer old', 'Bearer new', 'Bearer new']);
	await session.signOut();
});

test('404 (radio, remote or unknown) and 403 (not permitted) fail at once with a clear reason', async () => {
	const { m, server, sleeps } = setup([track(2)]);
	server.hooks.push((req) => (req.path.includes('/2/') ? new Response('', { status: 403 }) : undefined));
	await m.setUser('alice');
	m.enqueue([{ id: 1 }, { id: 2 }]);
	await settle(m);
	expect(m.item(1)!).toMatchObject({ status: 'failed', message: expect.stringContaining('Not available for download') });
	expect(m.item(2)!).toMatchObject({ status: 'failed', message: 'Your account isn’t allowed to download.' });
	expect(sleeps).toEqual([]);
	expect(server.reqs.every((r) => r.format === null)).toBe(true);
});

test('hostile track details are refused and nothing is stored', async () => {
	const { m, server, store } = setup([track(1)]);
	server.hooks.push((req) => (req.path.endsWith('/info') ? Response.json({ id: 1, key: '../../etc/passwd', original: { size: 1 }, formats: ['original'] }) : undefined));
	await m.setUser('alice');
	m.enqueue([{ id: 1 }]);
	await settle(m);
	expect(m.item(1)!).toMatchObject({ status: 'failed', message: 'The server sent unexpected track details.' });
	expect(server.reqs.filter((r) => r.format)).toEqual([]);
	expect(await store.listTracks('alice')).toEqual([]);
});

test('ids that are not positive integers never reach a URL', async () => {
	const { m, server } = setup([track(1)]);
	await m.setUser('alice');
	const n = m.enqueue([{ id: '1/../../admin' }, { id: '5?format=x' }, { id: -1 }, { id: 1.5 }, { id: { toString: () => '1' } }, { id: '../1' }]);
	expect(n).toBe(0);
	expect(m.items).toEqual([]);
	expect(server.reqs).toEqual([]);
});

test('the format follows the choice, the network default, and what the server offers', async () => {
	const tracks = [track(1), track(2, 1000, { formats: ['original'] }), track(3)];
	const { m, server } = setup(tracks, { defaultFormat: () => 'opus' });
	await m.setUser('alice');
	m.enqueue([{ id: 1 }]);
	m.enqueue([{ id: 2 }], 'mp3');
	m.enqueue([{ id: 3 }], 'mp3');
	await settle(m);
	expect(server.reqs.filter((r) => r.format).map((r) => `${r.path}:${r.format}`)).toEqual([
		'/v1/download/track/1:opus',
		'/v1/download/track/2:original',
		'/v1/download/track/3:mp3',
	]);
	expect(m.savedById(3)).toMatchObject({ format: 'mp3', contentType: 'audio/mpeg', ext: 'mp3' });
});

test('pause stops a download and keeps its bytes; resume carries on with Range', async () => {
	const t = track(1, 3 * CHUNK_BYTES);
	const { m, server, store } = setup([t]);
	let release!: () => void;
	const gate = new Promise<void>((r) => (release = r));
	let first = true;
	server.hooks.push((req) => {
		if (!req.format || !first) return undefined;
		first = false;
		let n = 0;
		return new Response(
			new ReadableStream<Uint8Array>({
				async pull(ctl) {
					if (n++ === 0) return ctl.enqueue(t.bytes.slice(0, CHUNK_BYTES));
					await gate;
					ctl.enqueue(t.bytes.slice(CHUNK_BYTES, CHUNK_BYTES + 10));
				},
			}),
			{ headers: { 'content-length': String(t.bytes.length) } },
		);
	});
	await m.setUser('alice');
	m.enqueue([{ id: 1 }], 'original');
	await vi.waitFor(async () => expect((await store.getPartial('alice', key(1)))?.received).toBe(CHUNK_BYTES));
	m.pause(1);
	expect(m.item(1)!.status).toBe('paused');
	release();
	expect((await store.loadQueue('alice'))[0]).toMatchObject({ id: 1, paused: true });
	m.resume(1);
	await settle(m);
	expect(m.item(1)!.status).toBe('done');
	expect(server.reqs.filter((r) => r.format).map((r) => r.range)).toEqual([null, `bytes=${CHUNK_BYTES}-`]);
	expect(await sameBytes(await store.getAudio('alice', key(1)), t.bytes)).toBe(true);
});

test('cancel stops a download and throws its bytes away', async () => {
	const t = track(1, 3 * CHUNK_BYTES);
	const { m, server, store } = setup([t]);
	server.hooks.push((req) => {
		if (!req.format) return undefined;
		let n = 0;
		return new Response(
			new ReadableStream<Uint8Array>({
				async pull(ctl) {
					if (n++ === 0) return ctl.enqueue(t.bytes.slice(0, CHUNK_BYTES));
					await new Promise(() => {});
				},
			}),
			{ headers: { 'content-length': String(t.bytes.length) } },
		);
	});
	await m.setUser('alice');
	m.enqueue([{ id: 1 }], 'original');
	await vi.waitFor(async () => expect((await store.getPartial('alice', key(1)))?.received).toBe(CHUNK_BYTES));
	await m.cancel(1);
	expect(m.items).toEqual([]);
	expect(await store.getPartial('alice', key(1))).toBeUndefined();
	expect(await store.backend.list('chunks', '')).toHaveLength(0);
	expect(await store.loadQueue('alice')).toEqual([]);
});

test('a track already downloaded is matched by key (even after a rescan) and not fetched again', async () => {
	const { m, server, store } = setup([track(1), track(50, 1000, { key: key(1) })]);
	await m.setUser('alice');
	m.enqueue([{ id: 1 }]);
	await settle(m);
	expect(m.enqueue([{ id: 1 }])).toBe(0); // known by id
	// After a rescan the same file has id 50.
	m.enqueue([{ id: 50 }]);
	await settle(m);
	expect(server.reqs.filter((r) => r.format).length).toBe(1);
	expect((await store.getTrack('alice', key(1)))?.id).toBe(50);
	expect(m.saved).toHaveLength(1);
});

test('downloads are kept per user: switching users stops the queue and hides the other’s', async () => {
	const t = track(1, 2 * CHUNK_BYTES);
	const { m, server, store } = setup([t, track(2)]);
	server.hooks.push((req) => {
		if (!req.path.endsWith('/1') || !req.format) return undefined;
		let n = 0;
		return new Response(
			new ReadableStream<Uint8Array>({
				async pull(ctl) {
					if (n++ === 0) return ctl.enqueue(t.bytes.slice(0, 10));
					await new Promise(() => {});
				},
			}),
			{ headers: { 'content-length': String(t.bytes.length) } },
		);
	});
	await m.setUser('alice');
	m.enqueue([{ id: 2 }]);
	await settle(m);
	m.enqueue([{ id: 1 }]);
	await vi.waitFor(() => expect(m.item(1)?.received).toBe(10));
	await m.setUser('bob');
	expect(m.saved).toEqual([]);
	expect(m.items).toEqual([]);
	expect(await store.listTracks('bob')).toEqual([]);
	expect((await store.listTracks('alice')).map((x) => x.id)).toEqual([2]);
	await m.setUser('alice');
	expect(m.saved.map((x) => x.id)).toEqual([2]);
});

test('offline (not signed in): downloads are listed but nothing is fetched', async () => {
	const { m, server, store } = setup([track(1)]);
	await store.saveQueue('alice', [{ id: 1, albumId: 7, choice: 'auto', paused: false }]);
	await m.setUser('alice', false);
	expect(m.items).toEqual([]);
	expect(m.enqueue([{ id: 1 }])).toBe(0);
	expect(server.reqs).toEqual([]);
	// Back online: the saved queue carries on.
	await m.setUser('alice', true);
	await settle(m);
	expect(m.item(1)!.status).toBe('done');
});

test('remove, remove album and remove all', async () => {
	const { m, store } = setup([track(1, 100, { album_id: 7 }), track(2, 100, { album_id: 8 }), track(3, 100, { album_id: 8 })]);
	await m.setUser('alice');
	m.enqueue([{ id: 1 }, { id: 2 }, { id: 3 }]);
	await settle(m);
	expect(m.usedBytes).toBe(300);
	await m.remove([key(1)]);
	expect(m.saved.map((t) => t.id).sort()).toEqual([2, 3]);
	await m.removeAlbum(8);
	expect(m.saved).toEqual([]);
	m.enqueue([{ id: 1 }]);
	await settle(m);
	await m.removeAll();
	expect(await store.listTracks('alice')).toEqual([]);
	expect(await store.backend.list('chunks', '')).toHaveLength(0);
	expect(await store.backend.list('covers', '')).toHaveLength(0);
});

test('a full disk fails the download with a clear reason', async () => {
	const { m, store } = setup([track(1)]);
	store.putChunk = async () => {
		throw new DOMException('full', 'QuotaExceededError');
	};
	await m.setUser('alice');
	m.enqueue([{ id: 1 }]);
	await settle(m);
	expect(m.item(1)!).toMatchObject({ status: 'failed', message: 'Not enough storage space on this device.' });
});

test('classify keeps 401 after renewal as a sign-in problem', async () => {
	const { m, server } = setup([track(1)], { authed: async () => { throw new ApiError(401, 'x'); } });
	await m.setUser('alice');
	m.enqueue([{ id: 1 }]);
	await settle(m);
	expect(m.item(1)!.message).toBe('Signed out. Sign in again to download.');
	expect(server.reqs).toEqual([]);
});
