// Two engines, one sound (GitHub issue #11): playing downloads on this device first pauses
// whatever it plays on the server's behalf (through LMS while in reach, held here otherwise),
// and the server's music starting on this device stops the downloads. Before this, a download
// started while the stream played left both audible, with the transport on the downloads and
// no way to reach the stream.
import type { StoredTrack } from '#lib/offline/types';

vi.mock('#lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
vi.mock('#lib/offline/index.svelte', async () => {
	const { DownloadManager } = await import('#lib/offline/manager.svelte');
	const { MemoryBackend, OfflineStore } = await import('#lib/offline/store');
	const { HandoffOffer } = await import('#lib/offline/handoff.svelte');
	const { PlaySync } = await import('#lib/offline/plays.svelte');
	const offlineStore = new OfflineStore(new MemoryBackend());
	const hang = (() => new Promise(() => {})) as typeof fetch;
	const downloads = new DownloadManager({ store: offlineStore, authed: (fn) => fn('t'), fetch: hang, sleep: async () => {}, waitForWifi: () => false, capBytes: () => 0 });
	const plays = new PlaySync({ store: offlineStore, authed: (fn) => fn('t'), fetch: hang });
	const { FakeOffline } = await import('./helpers/fake-offline.svelte');
	return { offlineStore, downloads, plays, offlinePlayer: new FakeOffline(), handoff: new HandoffOffer(), lookupTrack: async () => null };
});

const fake = await import('./helpers/fake-app.svelte');
const offline = await import('#lib/offline/index.svelte');
const engine = offline.offlinePlayer as unknown as { queue: unknown[]; playing: boolean; calls: string[] };
const { device } = await import('#lib/player/device.svelte');
const { controls, local } = await import('#lib/player/controls.svelte');

const P = '02:00:00:00:00:01';
const KITCHEN = '02:00:00:00:00:02';
const stored = (id: number): StoredTrack => ({
	user: 'alice', key: `k${id}`, id, title: `Kept ${id}`, artist: '', album: '', albumId: null, tracknum: null, duration: 180,
	format: 'mp3', codec: 'mp3', ext: 'mp3', size: 1, contentType: 'audio/mpeg', downloadedAt: 0, chunks: 1,
});
const sent = () => fake.calls.map((c) => `${c.player} ${c.cmd.join(' ')}`);

beforeEach(() => {
	fake.calls.length = 0;
	engine.calls.length = 0;
	engine.queue = [];
	engine.playing = false;
	fake.app.players = [
		{ playerid: P, name: 'This browser' },
		{ playerid: KITCHEN, name: 'Kitchen' },
	];
	fake.app.playerId = P;
	fake.app.reach = 'online';
	fake.app.now = { mode: 'play', time: 30, duration: 180, playlist_cur_index: 0, playlist_tracks: 2, playlist_loop: [{ id: 1, title: 'Streamed', duration: 180 }] };
	fake.app.elapsed = 30;
	device.mac = P;
	device.detail = { output: 'Playing', received: true };
	local.sync(false, true);
});

test('in reach, this device playing its stream: LMS is told to pause, then the downloads start', async () => {
	await controls.playDownloads('alice', [stored(1), stored(2)], 1);
	expect(sent()).toContain(`${P} pause 1`);
	expect(engine.calls).toEqual(['playList alice 2 1']);
	expect(engine.playing).toBe(true);
});

test('controlling another player while this device plays: this device alone is paused', async () => {
	fake.app.playerId = KITCHEN;
	await controls.playDownloads('alice', [stored(1)]);
	expect(sent().filter((s) => s.includes('pause'))).toEqual([`${P} pause 1`]);
	expect(engine.calls).toEqual(['playList alice 1 0']);
});

test('nothing audible here: nothing is paused', async () => {
	device.detail = { output: 'Paused' };
	await controls.playDownloads('alice', [stored(1)]);
	expect(sent().some((s) => s.includes('pause'))).toBe(false);
	// No player engine on this device at all (another player's controller).
	device.detail = { output: 'Playing' };
	device.mac = '';
	await controls.playDownloads('alice', [stored(1)]);
	expect(sent().some((s) => s.includes('pause'))).toBe(false);
	expect(engine.calls).toHaveLength(2);
});

test('out of reach: the stream is held here, and LMS is not told anything', async () => {
	fake.app.reach = 'unreachable';
	local.sync(true, false);
	const held = vi.spyOn(device, 'local').mockImplementation(() => {});
	await controls.playDownloads('alice', [stored(1)]);
	expect(held).toHaveBeenCalledWith({ hold: true });
	expect(sent().some((s) => s.includes('pause'))).toBe(false);
	expect(engine.calls).toEqual(['playList alice 1 0']);
	held.mockRestore();
	local.sync(false, true);
});

test("the server's music starting on this device stops the downloads; a pause to start them does not", () => {
	engine.queue = [stored(1)];
	engine.playing = true;
	// Paused (or stopped) and then playing: LMS has started a stream here.
	device.detail = { output: 'Paused' };
	device.applyStatus({ output: 'Playing' });
	expect(engine.calls).toContain('stop');
	expect(engine.queue).toEqual([]);
	// Already playing when the downloads begin (they pause it): no transition, no stop.
	engine.calls.length = 0;
	engine.queue = [stored(1)];
	device.applyStatus({ output: 'Playing', elapsed_ms: 1000 });
	device.applyStatus({ output: 'Playing', elapsed_ms: 2000, held: true });
	expect(engine.calls).not.toContain('stop');
	// Nothing queued: nothing to stop, and no error.
	engine.queue = [];
	device.detail = { output: 'Stopped' };
	expect(() => device.applyStatus({ output: 'Playing' })).not.toThrow();
	expect(engine.calls).not.toContain('stop');
});

test('the start-of-playing hook can be taken down', () => {
	let n = 0;
	const off = device.onPlaying(() => n++);
	device.detail = { output: 'Stopped' };
	device.applyStatus({ output: 'Playing' });
	expect(n).toBe(1);
	off();
	device.applyStatus({ output: 'Paused' });
	device.applyStatus({ output: 'Playing' });
	expect(n).toBe(1);
});
