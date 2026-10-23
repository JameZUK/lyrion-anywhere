// The offline player: a local queue in album order, transport, lock-screen controls, a clear
// message for formats this browser can't play, and every blob: URL given back.
import { OfflinePlayer, playable, unplayableMessage, type AudioLike } from '#lib/offline/player.svelte';
import { MemoryBackend, OfflineStore } from '#lib/offline/store';
import { key, stored } from './helpers/fake-downloads';
import type { StoredTrack } from '#lib/offline/types';

class FakeAudio extends EventTarget implements AudioLike {
	src = '';
	currentTime = 0;
	duration = NaN;
	paused = true;
	preload = '';
	plays = 0;
	supported = new Set(['audio/mpeg', 'audio/flac', 'audio/ogg; codecs=opus']);
	play() {
		this.plays++;
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
	canPlayType(t: string) {
		return this.supported.has(t) ? 'maybe' : '';
	}
	removeAttribute(n: string) {
		if (n === 'src') this.src = '';
	}
}

class FakeSession {
	handlers = new Map<string, MediaSessionActionHandler | null>();
	metadata: unknown = null;
	playbackState = 'none';
	positions: MediaPositionState[] = [];
	setActionHandler(a: string, fn: MediaSessionActionHandler | null) {
		this.handlers.set(a, fn);
	}
	setPositionState(p: MediaPositionState) {
		this.positions.push(p);
	}
}

async function setup(tracks: StoredTrack[]) {
	const store = new OfflineStore(new MemoryBackend());
	for (const t of tracks) {
		await store.putTrack(t);
		await store.putChunk(t.user, t.key, 0, new Blob([t.title]));
	}
	await store.putCover('alice', 7, new Blob(['png'], { type: 'image/png' }));
	const audio = new FakeAudio();
	const ms = new FakeSession();
	const live = new Set<string>();
	let n = 0;
	const urls = {
		create: vi.fn((_b: Blob) => {
			const u = `blob:test/${++n}`;
			live.add(u);
			return u;
		}),
		revoke: vi.fn((u: string) => live.delete(u)),
	};
	const p = new OfflinePlayer({ store, createAudio: () => audio, mediaSession: ms as unknown as MediaSession, urls, metadata: (init) => init as unknown as MediaMetadata });
	return { p, audio, ms, live, urls, store };
}

const album = [1, 2, 3].map((i) => stored({ key: key(i), id: i, title: `Song ${i}`, tracknum: i }));

test('plays an album in order: next, previous, ended and the last track', async () => {
	const { p, audio } = await setup(album);
	await p.playList('alice', album, 0);
	expect(p.current?.title).toBe('Song 1');
	expect(p.playing).toBe(true);
	expect(audio.src).toMatch(/^blob:/);
	await p.next();
	expect(p.index).toBe(1);
	audio.dispatchEvent(new Event('ended'));
	await vi.waitFor(() => expect(p.index).toBe(2));
	await p.next(); // nothing after the last
	expect(p.index).toBe(2);
	audio.dispatchEvent(new Event('ended'));
	await vi.waitFor(() => expect(p.playing).toBe(false));
	expect(p.index).toBe(2);
	// Previous: near the start goes back a track; later it restarts the track.
	await p.previous();
	expect(p.index).toBe(1);
	p.position = 30;
	audio.currentTime = 30;
	await p.previous();
	expect(p.index).toBe(1);
	expect(audio.currentTime).toBe(0);
});

test('seek, pause and resume', async () => {
	const { p, audio } = await setup(album);
	await p.playList('alice', album, 1);
	p.duration = 200;
	p.seek(42);
	expect(audio.currentTime).toBe(42);
	p.seek(9999);
	expect(audio.currentTime).toBe(200);
	p.seek(-5);
	expect(audio.currentTime).toBe(0);
	p.seek(NaN);
	expect(audio.currentTime).toBe(0);
	p.toggle();
	expect(p.playing).toBe(false);
	p.toggle();
	await vi.waitFor(() => expect(p.playing).toBe(true));
});

test('lock-screen controls: metadata, handlers that drive the player, and cleared on stop', async () => {
	const { p, audio, ms } = await setup(album);
	await p.playList('alice', album, 0);
	await vi.waitFor(() => expect(ms.metadata).toMatchObject({ title: 'Song 1', artist: 'Artist', album: 'Album' }));
	expect((ms.metadata as MediaMetadataInit).artwork?.[0]?.src).toMatch(/^blob:/);
	expect(ms.playbackState).toBe('playing');
	for (const a of ['play', 'pause', 'nexttrack', 'previoustrack', 'seekto', 'seekforward', 'seekbackward', 'stop']) expect(ms.handlers.get(a), a).toBeTypeOf('function');
	ms.handlers.get('nexttrack')!({ action: 'nexttrack' });
	await vi.waitFor(() => expect(p.index).toBe(1));
	ms.handlers.get('pause')!({ action: 'pause' });
	expect(audio.paused).toBe(true);
	expect(ms.playbackState).toBe('paused');
	p.duration = 100;
	ms.handlers.get('seekto')!({ action: 'seekto', seekTime: 50 });
	expect(audio.currentTime).toBe(50);
	ms.handlers.get('seekforward')!({ action: 'seekforward' });
	expect(audio.currentTime).toBe(60);
	p.stop();
	for (const a of ms.handlers.keys()) expect(ms.handlers.get(a), a).toBeNull();
	expect(ms.metadata).toBeNull();
	expect(ms.playbackState).toBe('none');
	expect(p.active).toBe(false);
});

test('a format this browser can’t play says so and suggests MP3 or Opus', async () => {
	const flac = stored({ key: key(9), title: 'Lossless', format: 'original', codec: 'flc', ext: 'flac', contentType: 'audio/x-flac' });
	const { p, audio, urls } = await setup([flac]);
	audio.supported = new Set(['audio/mpeg']);
	await p.playList('alice', [flac], 0);
	expect(p.error).toBe('This browser can’t play FLC files. Remove the download and download it again as MP3 or Opus to play it here.');
	expect(audio.plays).toBe(0);
	await vi.waitFor(() => expect(p.cover).not.toBe(''));
	expect(urls.create).toHaveBeenCalledTimes(1); // the cover only, no audio
	// Known aliases are tried: audio/x-flac fails but audio/flac plays.
	audio.supported = new Set(['audio/flac']);
	expect(playable(audio, flac)).toBe(true);
	expect(unplayableMessage({ format: 'opus', codec: 'opus', ext: 'opus' })).toContain('Opus files');
});

test('a missing download says so instead of failing silently', async () => {
	const { p, store } = await setup(album);
	await store.deleteTrack('alice', key(2));
	await p.playList('alice', album, 1);
	expect(p.error).toContain('missing or incomplete');
	expect(p.playing).toBe(false);
});

test('every blob: URL is given back (track changes, stop)', async () => {
	const { p, live, urls } = await setup(album);
	await p.playList('alice', album, 0);
	await p.next();
	await p.next();
	await vi.waitFor(() => expect(p.cover).not.toBe(''));
	expect(live.size).toBe(2); // the current track and the cover
	p.stop();
	expect(live.size).toBe(0);
	expect(urls.revoke.mock.calls.length).toBe(urls.create.mock.calls.length);
});

test('another user’s downloads are not readable through the player', async () => {
	const { p, store } = await setup(album);
	await store.putTrack(stored({ user: 'bob', key: key(1), title: 'Bob’s' }));
	await p.playList('bob', album, 0); // bob has the record but not alice's audio
	expect(p.error).toContain('missing or incomplete');
});

test('removing the playing track stops playback; others are dropped from the queue', async () => {
	const { p } = await setup(album);
	await p.playList('alice', album, 1);
	p.forget([key(3)]);
	expect(p.queue.map((t) => t.key)).toEqual([key(1), key(2)]);
	expect(p.index).toBe(1);
	p.forget([key(2)]);
	expect(p.active).toBe(false);
});
