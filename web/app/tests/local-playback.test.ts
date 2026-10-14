// This browser as the player while the server is out of reach (lib/player/local.svelte):
// pause/resume on the device's own stream, skipping to downloads and back, carrying on at a
// track's end, and what LMS is told when it is back (only if its queue still matches).
import type { Command } from '$lib/api/rpc';
import type { PlayerStatus, Track } from '$lib/lms/types';
import type { StoredTrack } from '$lib/offline/types';
import { LocalPlayback, type LocalAudio, type LocalDeps } from '$lib/player/local.svelte';
import { NEXT_NEEDS_SERVER } from '$lib/player/local-plan';

class FakeAudio extends EventTarget {
	src = '';
	currentTime = 0;
	duration = 200;
	paused = true;
	ended = false;
	preload = '';
	volume = 1;
	playError: Error | null = null;
	async play() {
		if (this.playError) throw this.playError;
		this.paused = false;
		this.dispatchEvent(new Event('play'));
	}
	pause() {
		if (this.paused) return;
		this.paused = true;
		this.dispatchEvent(new Event('pause'));
	}
	load() {}
	canPlayType(t: string) {
		return t.includes('flac') || t.includes('mpeg') ? 'maybe' : '';
	}
	removeAttribute(n: string) {
		if (n === 'src') this.src = '';
	}
	end() {
		this.ended = true;
		this.paused = true;
		this.dispatchEvent(new Event('ended'));
	}
}

const MAC = '02:00:00:00:00:01';
const row = (id: number): Track => ({ id, title: `Song ${id}`, duration: 200 });
const stored = (id: number, over: Partial<StoredTrack> = {}): StoredTrack => ({
	user: 'u',
	key: `k${id}`,
	id,
	title: `Song ${id}`,
	artist: '',
	album: '',
	albumId: null,
	tracknum: null,
	duration: 200,
	format: 'original',
	codec: 'flc',
	ext: 'flac',
	size: 1,
	contentType: 'audio/flac',
	downloadedAt: 0,
	chunks: 1,
	...over,
});

function setup(opts: { downloads?: number[]; status?: PlayerStatus; lms?: (c: Command) => unknown } = {}) {
	const audio = new FakeAudio();
	const dev = { output: 'Playing', held: false, received: false, track: 1 as number | null, fresh: false, msgs: [] as unknown[] };
	let now: PlayerStatus | null = opts.status ?? { mode: 'play', playlist_cur_index: 1, playlist_tracks: 4, time: 30, playlist_loop: [row(11)] };
	let elapsed = Number(now.time ?? 0);
	const sent: { player: string; c: Command }[] = [];
	const downloads = new Map((opts.downloads ?? []).map((id) => [id, stored(id)]));
	const deps: LocalDeps = {
		device: {
			mac: () => MAC,
			output: () => dev.output,
			held: () => dev.held,
			received: () => dev.received,
			track: () => dev.track,
			fresh: () => dev.fresh,
			local: (m) => {
				dev.msgs.push(m);
				if ('hold' in m) dev.held = true;
				else dev.held = false;
			},
		},
		status: () => now,
		apply: (p) => {
			now = { ...now!, ...p };
			if (typeof p.time === 'number') elapsed = p.time;
		},
		elapsed: () => elapsed,
		downloaded: (id) => (typeof id === 'number' ? downloads.get(id) : undefined),
		audio: async () => new Blob(['x']),
		rpc: async <T,>(player: string, c: Command) => {
			sent.push({ player, c });
			return ((opts.lms ?? (() => ({})))(c) ?? {}) as T;
		},
		refreshed: () => {},
		createAudio: () => audio as unknown as LocalAudio,
		urls: { create: () => 'blob:x', revoke: () => {} },
	};
	const p = new LocalPlayback(deps);
	p.rows = new Map([0, 1, 2, 3].map((i) => [i, row(10 + i)]));
	return { p, dev, audio, sent, get now() { return now!; }, setElapsed: (t: number) => (elapsed = t) };
}
const flush = () => new Promise((r) => setTimeout(r, 0));

test('engaging takes over where LMS was, on the device’s own stream', () => {
	const t = setup();
	t.p.sync(true, false);
	expect(t.p).toMatchObject({ engaged: true, index: 1, source: 'stream', paused: false, waiting: '' });
	// Nothing done: back in reach, nothing is told, and the output is left alone.
	t.p.sync(false, true);
	expect(t.sent).toEqual([]);
	expect(t.dev.msgs).toEqual([]);
});

test('pause and resume hold the output and keep the position', () => {
	const t = setup();
	t.p.sync(true, false);
	t.p.toggle();
	expect(t.dev.msgs).toEqual([{ hold: true }]);
	expect(t.now.mode).toBe('pause');
	expect(t.p.paused).toBe(true);
	t.p.toggle();
	expect(t.dev.msgs.at(-1)).toEqual({ release: 'now' });
	expect(t.now.mode).toBe('play');
	expect(t.now.time).toBe(30); // where it stopped
});

test('next to a download and back to the held stream', async () => {
	const t = setup({ downloads: [12] });
	t.p.sync(true, false);
	expect(t.p.available(2)).toBe(true);
	expect(t.p.available(3)).toBe(false);
	t.setElapsed(55);
	t.p.next();
	await flush();
	expect(t.p).toMatchObject({ index: 2, source: 'download' });
	expect(t.dev.msgs).toEqual([{ hold: true }]); // the stream waits, held, where it was
	expect(t.audio.src).toBe('blob:x');
	expect(t.audio.paused).toBe(false);
	expect(t.now.playlist_cur_index).toBe(2);
	expect(t.now.playlist_loop?.[0]?.id).toBe(12);
	// Previous within the first seconds: back to the stream, resumed where it was left.
	t.setElapsed(1);
	t.p.previous();
	expect(t.p).toMatchObject({ index: 1, source: 'stream' });
	expect(t.dev.msgs.at(-1)).toEqual({ release: 'now' });
	expect(t.now.time).toBe(55);
	expect(t.audio.src).toBe('');
});

test('nothing to skip to: next and jumps do nothing', () => {
	const t = setup();
	t.p.sync(true, false);
	t.p.next();
	t.p.jump(3);
	t.p.jump(-1);
	t.p.jump(Number.NaN);
	expect(t.p.index).toBe(1);
	expect(t.dev.msgs).toEqual([]);
});

test('a download that ends goes on to the next download; then waits, saying why', async () => {
	const t = setup({ downloads: [12, 13] });
	t.p.sync(true, false);
	t.p.jump(2);
	await flush();
	t.audio.end();
	await flush();
	expect(t.p).toMatchObject({ index: 3, source: 'download' });
	t.audio.end();
	await flush();
	expect(t.p.waiting).toBe(NEXT_NEEDS_SERVER);
	expect(t.now.mode).toBe('stop');
});

test('the device’s stream playing out goes on to a download', async () => {
	const t = setup({ downloads: [12] });
	t.p.sync(true, false);
	t.dev.output = 'Stopped';
	t.p.onDevice();
	await flush();
	expect(t.p).toMatchObject({ index: 2, source: 'download' });
	// …and the stream is gone, so it can't be gone back to.
	expect(t.p.available(1)).toBe(false);
});

test('the device moving on into a next track it already had follows along', () => {
	const t = setup();
	t.p.sync(true, false);
	t.dev.track = 2;
	t.p.onDevice();
	expect(t.p.index).toBe(2);
	expect(t.now.playlist_loop?.[0]?.id).toBe(12);
});

test('seeking works within a download only', async () => {
	const t = setup({ downloads: [12] });
	t.p.sync(true, false);
	t.p.seek(20);
	expect(t.now.time).toBe(30); // the stream: no
	t.p.jump(2);
	await flush();
	t.p.seek(80);
	expect(t.audio.currentTime).toBe(80);
	t.p.seek(Number.NaN);
	t.p.seek(-5);
	expect(t.audio.currentTime).toBe(0);
});

test('a download this browser can’t play is not available', () => {
	const t = setup({ downloads: [] });
	t.p.sync(true, false);
	(t.p as unknown as { deps: LocalDeps }).deps.downloaded = (id) => (id === 12 ? stored(12, { contentType: 'audio/x-ms-wma', ext: 'wma', codec: 'wma' }) : undefined);
	expect(t.p.available(2)).toBe(false);
});

describe('back in reach', () => {
	const lms = (track: number, index = 1, mode = 'play') => (c: Command) =>
		c[0] === 'status' && c[1] === '-' ? { mode, playlist_cur_index: index, playlist_tracks: 4 } : { playlist_loop: [{ id: track }] };

	test('LMS still has the same track there: told where this device is, then the output follows', async () => {
		const t = setup({ downloads: [12], lms: lms(12) });
		t.p.sync(true, false);
		t.p.jump(2);
		await flush();
		t.audio.currentTime = 64.2;
		t.p.sync(false, true);
		await flush();
		await flush();
		const cmds = t.sent.filter((s) => s.c[0] !== 'status').map((s) => s.c);
		expect(cmds).toEqual([['playlist', 'index', 2], ['time', 64]]);
		expect(t.sent.every((s) => s.player === MAC)).toBe(true);
		expect(t.dev.msgs.at(-1)).toEqual({ release: 'flush' });
		expect(t.audio.src).toBe('');
	});

	test('paused here: LMS is paused there too', async () => {
		const t = setup({ lms: lms(11) });
		t.p.sync(true, false);
		t.p.toggle();
		t.p.sync(false, true);
		await flush();
		await flush();
		expect(t.sent.map((s) => s.c).filter((c) => c[0] === 'pause')).toEqual([['pause', 1]]);
		expect(t.sent.some((s) => s.c[0] === 'playlist')).toBe(false); // same stream: no restart
		expect(t.dev.msgs.at(-1)).toEqual({ release: 'paused' });
	});

	test('a fresh session (LMS saw a new player): the position is retold', async () => {
		const t = setup({ lms: lms(11) });
		t.p.sync(true, false);
		t.p.toggle();
		t.dev.fresh = true;
		t.p.sync(false, true);
		await flush();
		await flush();
		expect(t.sent.map((s) => s.c).filter((c) => c[0] !== 'status')).toEqual([['playlist', 'index', 1], ['time', 30], ['pause', 1]]);
	});

	test('someone changed the queue meanwhile: nothing is sent, LMS is followed', async () => {
		const t = setup({ downloads: [12], lms: lms(99) });
		t.p.sync(true, false);
		t.p.jump(2);
		await flush();
		t.p.sync(false, true);
		await flush();
		await flush();
		expect(t.sent.map((s) => s.c).filter((c) => c[0] !== 'status')).toEqual([]);
		expect(t.dev.msgs.at(-1)).toEqual({ release: 'now' });
	});

	test('stopped at the end for want of the next track: the server plays it', async () => {
		const t = setup({ downloads: [12], lms: lms(13, 1) });
		t.p.sync(true, false);
		t.p.jump(2);
		await flush();
		t.audio.end();
		await flush();
		t.p.sync(false, true);
		await flush();
		await flush();
		expect(t.sent.map((s) => s.c).filter((c) => c[0] !== 'status')).toEqual([['playlist', 'index', 3]]);
	});

	test('the server failing again while being told: the output is given back anyway', async () => {
		const t = setup({ downloads: [12], lms: () => { throw new TypeError('Failed to fetch'); } });
		t.p.sync(true, false);
		t.p.jump(2);
		await flush();
		t.p.sync(false, true);
		await flush();
		await flush();
		expect(t.dev.msgs.at(-1)).toEqual({ release: 'now' });
		expect(t.p.source).toBeNull();
	});

	test('another player chosen while out of reach: this device stops, and the server is told later', async () => {
		const t = setup({ downloads: [12], lms: lms(12) });
		t.p.sync(true, false);
		t.p.jump(2);
		await flush();
		t.audio.currentTime = 12;
		t.p.sync(false, false); // not this device any more, still out of reach
		expect(t.audio.src).toBe('');
		expect(t.sent).toEqual([]);
		t.p.sync(false, true); // back
		await flush();
		await flush();
		expect(t.sent.map((s) => s.c).filter((c) => c[0] !== 'status')).toEqual([['playlist', 'index', 2], ['time', 12], ['pause', 1]]);
	});
});

describe('marks', () => {
	test('downloads for any player; the stream held whole only for this device', () => {
		const t = setup({ downloads: [13] });
		expect(t.p.mark(3, 13, false, 1)).toBe('downloaded');
		expect(t.p.mark(1, 11, true, 1)).toBeNull(); // not all received
		t.dev.received = true;
		expect(t.p.mark(1, 11, true, 1)).toBe('device');
		expect(t.p.mark(1, 11, false, 1)).toBeNull();
		expect(t.p.mark(2, 12, true, 1)).toBeNull();
		t.dev.output = 'Stopped';
		expect(t.p.mark(1, 11, true, 1)).toBeNull();
	});
});
