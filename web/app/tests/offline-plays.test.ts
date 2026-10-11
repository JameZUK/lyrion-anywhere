// Offline play sync-back (SPEC D6): plays queued per user in storage, sent in batches to
// POST /v1/plays when online, kept until the server has answered for them, with backoff.
// Also: the offline player reports what was actually heard.
import { MemoryBackend, OfflineStore } from '$lib/offline/store';
import { BATCH, MAX_AGE_S, PlaySync, body, parseResults, playBackoff } from '$lib/offline/plays.svelte';
import { OfflinePlayer, type AudioLike } from '$lib/offline/player.svelte';
import { ApiError } from '$lib/api/http';
import { key, stored } from './helpers/fake-downloads';

const NOW = 1_790_000_000_000;

interface Sent {
	auth: string;
	client: string;
	items: Record<string, unknown>[];
}

/** A fake /v1/plays: answers each play with `answer(item)` (recorded by default). */
function fakeServer() {
	const sent: Sent[] = [];
	const seen = new Set<string>();
	const s = {
		sent,
		status: 200,
		headers: {} as Record<string, string>,
		raw: undefined as unknown,
		fail: false,
		answer: (item: Record<string, unknown>): Record<string, unknown> => {
			const k = String(item.key);
			if (seen.has(k)) return { key: k, result: 'duplicate' };
			seen.add(k);
			return { key: k, result: 'recorded' };
		},
		fetch: (async (url: string, init?: RequestInit) => {
			expect(url).toBe('/v1/plays');
			expect(init?.method).toBe('POST');
			const h = init?.headers as Record<string, string>;
			const items = JSON.parse(String(init?.body)) as Record<string, unknown>[];
			sent.push({ auth: h.Authorization ?? '', client: h['X-Anywhere-Client'] ?? '', items });
			if (s.fail) throw new TypeError('network down');
			if (s.status !== 200) return new Response('{"error":"x"}', { status: s.status, headers: s.headers });
			const out = s.raw !== undefined ? s.raw : { results: items.map((i) => s.answer(i)) };
			return new Response(JSON.stringify(out), { status: 200, headers: { 'content-type': 'application/json' } });
		}) as unknown as typeof fetch,
	};
	return s;
}

function setup(over: Partial<ConstructorParameters<typeof PlaySync>[0]> = {}) {
	const store = new OfflineStore(new MemoryBackend());
	const server = fakeServer();
	const timers: { fn: () => void; ms: number }[] = [];
	let n = 0;
	const sync = new PlaySync({
		store,
		authed: (fn) => fn('tok'),
		fetch: (...a) => server.fetch(...a),
		now: () => NOW,
		uuid: () => `00000000-0000-4000-8000-${String(++n).padStart(12, '0')}`,
		setTimer: (fn, ms) => {
			timers.push({ fn, ms });
			return timers.length;
		},
		clearTimer: () => {},
		...over,
	});
	return { store, server, sync, timers };
}

const T1 = { id: 11, key: key(1) };
const ago = (s: number) => NOW - s * 1000;
const settle = () => vi.waitFor(() => new Promise((r) => setTimeout(r, 0)));

async function idle(sync: PlaySync) {
	// Let the background send loop finish.
	for (let i = 0; i < 20; i++) await settle();
	return sync;
}

test('plays are queued per user and sent with the right shape', async () => {
	const { store, server, sync } = setup();
	await sync.setUser('alice', false);
	await sync.record('alice', T1, ago(600), 250.04);
	await sync.record('alice', T1, ago(300), 30);
	await sync.record('bob', { id: 12, key: key(2) }, ago(300), 250);
	expect(sync.pending).toBe(2);
	expect(server.sent).toEqual([]); // offline: nothing sent
	const queued = await store.listPlays('alice');
	expect(queued.map((p) => p.playedAt)).toEqual([NOW / 1000 - 600, NOW / 1000 - 300]);
	expect(queued[0]!.id).toMatch(/^\d{11}-[0-9a-f-]{36}$/);

	await sync.setUser('alice', true);
	await idle(sync);
	expect(server.sent.length).toBe(1);
	expect(server.sent[0]!.auth).toBe('Bearer tok');
	expect(server.sent[0]!.client).toBe('1');
	expect(server.sent[0]!.items[0]).toEqual({ key: queued[0]!.id, track_id: 11, played_at: NOW / 1000 - 600, seconds: 250, track_key: key(1) });
	expect(Object.keys(server.sent[0]!.items[0]!).sort()).toEqual(['key', 'played_at', 'seconds', 'track_id', 'track_key']);
	expect(sync.pending).toBe(0);
	expect(await store.listPlays('alice')).toEqual([]);
	// Bob's plays wait for Bob.
	expect((await store.listPlays('bob')).length).toBe(1);
	expect(server.sent.flatMap((s) => s.items).some((i) => i.track_id === 12)).toBe(false);
});

test('recorded, duplicate and refused plays are done; retry keeps them and waits', async () => {
	const { store, server, sync, timers } = setup();
	await sync.setUser('alice', false);
	for (let i = 0; i < 4; i++) await sync.record('alice', T1, ago(1000 - i), 250);
	const ids = (await store.listPlays('alice')).map((p) => p.id);
	const verdicts = ['recorded', 'duplicate', 'refused', 'retry'];
	server.answer = (item) => {
		const r = verdicts[ids.indexOf(String(item.key))]!;
		return { key: item.key, result: r, ...(r === 'refused' ? { reason: 'too_short' } : r === 'retry' ? { reason: 'rate_limited' } : {}) };
	};
	await sync.setUser('alice', true);
	await idle(sync);
	expect((await store.listPlays('alice')).map((p) => p.id)).toEqual([ids[3]]);
	expect(sync.pending).toBe(1);
	expect(timers.at(-1)?.ms).toBe(60_000);
	// When the timer fires, the play is sent again and, now accepted, goes.
	server.answer = (item) => ({ key: item.key, result: 'recorded' });
	timers.at(-1)!.fn();
	await idle(sync);
	expect(await store.listPlays('alice')).toEqual([]);
	expect(server.sent.length).toBe(2);
	expect(server.sent[1]!.items.map((i) => i.key)).toEqual([ids[3]]);
});

test('large queues go in batches of at most 200', async () => {
	const { store, server, sync } = setup();
	await sync.setUser('alice', false);
	for (let i = 0; i < 450; i++) await sync.record('alice', T1, ago(100_000 - i), 250);
	await sync.setUser('alice', true);
	await idle(sync);
	expect(server.sent.map((s) => s.items.length)).toEqual([BATCH, BATCH, 50]);
	expect(await store.listPlays('alice')).toEqual([]);
	// Oldest first, each sent once.
	const keys = server.sent.flatMap((s) => s.items.map((i) => String(i.key)));
	expect(new Set(keys).size).toBe(450);
	expect([...keys].sort()).toEqual(keys);
});

test('failures keep every play and back off; Retry-After is honoured', async () => {
	const { store, server, sync, timers } = setup();
	await sync.setUser('alice', false);
	await sync.record('alice', T1, ago(600), 250);
	server.fail = true;
	await sync.setUser('alice', true);
	await idle(sync);
	expect((await store.listPlays('alice')).length).toBe(1);
	expect(timers.at(-1)?.ms).toBe(playBackoff(1));
	timers.at(-1)!.fn();
	await idle(sync);
	expect(timers.at(-1)?.ms).toBe(playBackoff(2));
	expect(playBackoff(2)).toBeGreaterThan(playBackoff(1));
	expect(playBackoff(50)).toBe(600_000);

	server.fail = false;
	server.status = 503;
	timers.at(-1)!.fn();
	await idle(sync);
	expect(timers.at(-1)?.ms).toBe(playBackoff(3));
	server.status = 429;
	server.headers = { 'retry-after': '7' };
	timers.at(-1)!.fn();
	await idle(sync);
	expect(timers.at(-1)?.ms).toBe(7000);
	expect((await store.listPlays('alice')).length).toBe(1);

	// A malformed answer is a failure too: nothing is dropped.
	server.status = 200;
	for (const raw of [{}, { results: [] }, { results: [{ result: 'hacked' }] }, [], 'x', { results: [{}, {}] }]) {
		server.raw = raw;
		timers.at(-1)!.fn();
		await idle(sync);
		expect((await store.listPlays('alice')).length).toBe(1);
	}
	server.raw = undefined;
	timers.at(-1)!.fn();
	await idle(sync);
	expect(await store.listPlays('alice')).toEqual([]);
});

test('signed out (401) stops sending and keeps the plays', async () => {
	const { store, server, sync, timers } = setup({ authed: () => Promise.reject(new ApiError(401, 'signed out')) });
	await sync.setUser('alice', false);
	await sync.record('alice', T1, ago(600), 250);
	await sync.setUser('alice', true);
	await idle(sync);
	expect(server.sent).toEqual([]);
	expect(timers).toEqual([]);
	expect((await store.listPlays('alice')).length).toBe(1);
});

test('a batch the server won’t read is split until the bad play is found and dropped', async () => {
	const { store, server, sync } = setup();
	await sync.setUser('alice', false);
	for (let i = 0; i < 5; i++) await sync.record('alice', T1, ago(1000 - i), 250);
	const bad = (await store.listPlays('alice'))[2]!.id;
	const real = server.fetch;
	server.fetch = (async (url: string, init?: RequestInit) => {
		const items = JSON.parse(String(init?.body)) as { key: string }[];
		if (items.some((i) => i.key === bad)) {
			server.sent.push({ auth: '', client: '', items });
			return new Response('{"error":"malformed plays"}', { status: 400 });
		}
		return real(url, init);
	}) as unknown as typeof fetch;
	await sync.setUser('alice', true);
	await idle(sync);
	expect(await store.listPlays('alice')).toEqual([]);
	const recorded = server.sent.filter((s) => !s.items.some((i) => i.key === bad)).flatMap((s) => s.items);
	expect(recorded.length).toBe(4);
});

test('nothing invalid or hopeless is queued or sent', async () => {
	const { store, server, sync } = setup();
	await sync.setUser('alice', false);
	await sync.record('alice', T1, ago(600), 0.5); // skipped at once
	await sync.record('alice', T1, ago(600), Number.NaN);
	await sync.record('alice', { id: 0, key: key(1) }, ago(600), 250);
	await sync.record('alice', { id: 1.5, key: key(1) }, ago(600), 250);
	await sync.record('alice', { id: 11, key: '../x' }, ago(600), 250);
	await sync.record('', T1, ago(600), 250);
	await sync.record('alice', T1, Number.NaN, 250);
	expect(await store.listPlays('alice')).toEqual([]);
	// Absurd lengths are capped at a day.
	await sync.record('alice', T1, ago(600), 1e9);
	expect((await store.listPlays('alice'))[0]!.seconds).toBe(86_400);
	// Too old for the server: dropped without being sent.
	await sync.record('alice', T1, NOW - (MAX_AGE_S + 10) * 1000, 250);
	await sync.setUser('alice', true);
	await idle(sync);
	expect(server.sent.flatMap((s) => s.items).length).toBe(1);
	expect(await store.listPlays('alice')).toEqual([]);
	// Ids from storage are checked before use.
	await expect(store.deletePlay('alice', '../../tracks/x')).rejects.toThrow(RangeError);
	await expect(store.addPlay({ user: 'alice', id: 'nope', trackId: 1, trackKey: key(1), playedAt: 1, seconds: 1 })).rejects.toThrow(RangeError);
});

test('the queue is bounded: the oldest plays go first', async () => {
	const { store, sync } = setup({ maxQueued: 5 });
	await sync.setUser('alice', false);
	for (let i = 0; i < 8; i++) await sync.record('alice', T1, ago(1000 - i), 250);
	const left = await store.listPlays('alice');
	expect(left.length).toBe(5);
	expect(left[0]!.playedAt).toBe(NOW / 1000 - 1000 + 3);
	expect(sync.pending).toBe(5);
});

test('removing a user’s downloads keeps their waiting plays', async () => {
	const { store, sync } = setup();
	await sync.record('alice', T1, ago(600), 250);
	await store.deleteUser('alice');
	expect((await store.listPlays('alice')).length).toBe(1);
});

test('answers are checked strictly', () => {
	expect(parseResults({ results: [{ key: 'k', result: 'recorded' }] }, 1)).toEqual([{ key: 'k', result: 'recorded' }]);
	expect(parseResults({ results: [{ key: null, result: 'refused', reason: 'invalid' }] }, 1)[0]!.reason).toBe('invalid');
	for (const bad of [null, [], { results: {} }, { results: [] }, { results: [{ result: 'ok' }] }, { results: [null] }, { results: [{ result: 'recorded' }, { result: 'recorded' }] }]) {
		expect(() => parseResults(bad, 1)).toThrow();
	}
	expect(body([{ user: 'a', id: '01790000000-00000000-0000-4000-8000-000000000001', trackId: 3, trackKey: key(3), playedAt: 1_790_000_000, seconds: 12.345 }])).toEqual([
		{ key: '01790000000-00000000-0000-4000-8000-000000000001', track_id: 3, played_at: 1_790_000_000, seconds: 12.3, track_key: key(3) },
	]);
});

// ---------------------------------------------------------------------------
// The offline player reports plays
// ---------------------------------------------------------------------------

class FakeAudio extends EventTarget implements AudioLike {
	src = '';
	currentTime = 0;
	duration = NaN;
	paused = true;
	preload = '';
	play() {
		this.paused = false;
		this.dispatchEvent(new Event('play'));
		return Promise.resolve();
	}
	pause() {
		if (this.paused) return;
		this.paused = true;
		this.dispatchEvent(new Event('pause'));
	}
	load() {}
	canPlayType() {
		return 'maybe';
	}
	removeAttribute(n: string) {
		if (n === 'src') this.src = '';
	}
	/** Play on for `s` seconds, in quarter-second steps. */
	advance(s: number) {
		for (let t = 0; t < s * 4; t++) {
			this.currentTime += 0.25;
			this.dispatchEvent(new Event('timeupdate'));
		}
	}
	jump(to: number) {
		this.currentTime = to;
		this.dispatchEvent(new Event('timeupdate'));
	}
}

async function player() {
	const store = new OfflineStore(new MemoryBackend());
	const album = [1, 2, 3].map((i) => stored({ key: key(i), id: i, title: `Song ${i}`, tracknum: i }));
	for (const t of album) {
		await store.putTrack(t);
		await store.putChunk(t.user, t.key, 0, new Blob([t.title]));
	}
	const audio = new FakeAudio();
	const plays: { user: string; id: number; started: number; seconds: number }[] = [];
	let now = NOW;
	const p = new OfflinePlayer({
		store,
		createAudio: () => audio,
		mediaSession: null,
		urls: { create: () => 'blob:x', revoke: () => {} },
		now: () => now,
		onPlay: (user, t, started, seconds) => plays.push({ user, id: t.id, started, seconds }),
	});
	return { p, audio, plays, album, tick: (ms: number) => (now += ms) };
}

test('the offline player reports what was heard: finished, skipped and stopped', async () => {
	const { p, audio, plays, album, tick } = await player();
	await p.playList('alice', album, 0);
	audio.advance(200);
	tick(200_000);
	audio.dispatchEvent(new Event('ended'));
	await vi.waitFor(() => expect(p.current?.id === 2 && p.playing).toBe(true));
	expect(plays).toEqual([{ user: 'alice', id: 1, started: NOW, seconds: 200 }]);

	// Seeking forward isn't listening.
	audio.currentTime = 0;
	audio.advance(10);
	audio.jump(150);
	audio.advance(5);
	await p.next(); // skipped
	expect(plays[1]).toMatchObject({ id: 2, seconds: 15, started: NOW + 200_000 });

	// Paused time isn't listening either.
	audio.currentTime = 0;
	audio.advance(3);
	p.pause();
	audio.advance(50);
	p.stop();
	expect(plays[2]).toMatchObject({ id: 3, seconds: 3 });
	expect(plays.length).toBe(3);
	// Stopping again reports nothing more.
	p.stop();
	expect(plays.length).toBe(3);
});

test('a track skipped before anything was heard is not reported', async () => {
	const { p, plays, album } = await player();
	await p.playList('alice', album, 0);
	await p.next();
	await p.next();
	expect(plays).toEqual([]);
});

test('a failing reporter never stops playback', async () => {
	const store = new OfflineStore(new MemoryBackend());
	const t = stored({ key: key(1), id: 1 });
	await store.putTrack(t);
	await store.putChunk('alice', t.key, 0, new Blob(['x']));
	const audio = new FakeAudio();
	const p = new OfflinePlayer({
		store,
		createAudio: () => audio,
		mediaSession: null,
		urls: { create: () => 'blob:x', revoke: () => {} },
		onPlay: () => {
			throw new Error('storage broke');
		},
	});
	await p.playList('alice', [t, stored({ key: key(2), id: 2 })], 0);
	audio.advance(5);
	await expect(p.next()).resolves.toBeUndefined();
	expect(p.current?.id).toBe(2);
});
