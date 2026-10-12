// Endless stations: when to refill, the commands sent, and when to stop.
import { BATCH, LOW_WATER, StationRunner, batchCommands, currentTrack, needsRefill, queuedIds, remaining } from '$lib/stations/refill';
import type { NextBatch, NextRequest } from '$lib/api/me';
import { ID_LIST_MAX } from '$lib/lms/commands';

const P = '02:4c:41:00:00:01';

test('tracks left after the current one', () => {
	expect(remaining({ playlist_tracks: 10, playlist_cur_index: 3 })).toBe(6);
	expect(remaining({ playlist_tracks: '10', playlist_cur_index: '9' })).toBe(0);
	expect(remaining({ playlist_tracks: 0 })).toBe(0);
	expect(remaining({})).toBeNull();
	expect(remaining(null)).toBeNull();
	expect(remaining({ playlist_tracks: 'x' })).toBeNull();
	expect(needsRefill({ playlist_tracks: 10, playlist_cur_index: 5 })).toBe(true); // 4 left
	expect(needsRefill({ playlist_tracks: 10, playlist_cur_index: 4 })).toBe(false); // 5 left
	expect(needsRefill({})).toBe(false);
	expect(LOW_WATER).toBe(5);
});

test('batches load first, then add, in lists that fit the gateway limit', () => {
	expect(batchCommands([1, 2, 3], 'load')).toEqual([['playlistcontrol', 'cmd:load', 'track_id:1,2,3']]);
	expect(batchCommands([1, 2], 'add')).toEqual([['playlistcontrol', 'cmd:add', 'track_id:1,2']]);
	const many = Array.from({ length: 50 }, (_, i) => 1_000_000 + i);
	const cmds = batchCommands(many, 'load');
	expect(cmds.length).toBeGreaterThan(1);
	expect(cmds[0]![1]).toBe('cmd:load');
	expect(cmds.slice(1).every((c) => c[1] === 'cmd:add')).toBe(true);
	for (const c of cmds) expect(String(c[2]).length - 'track_id:'.length).toBeLessThanOrEqual(ID_LIST_MAX);
	expect(cmds.flatMap((c) => String(c[2]).slice(9).split(',').map(Number))).toEqual(many);
	// Hostile ids from a bad answer are dropped, never sent.
	expect(batchCommands([1, -2, 1.5, NaN, 3], 'add')).toEqual([['playlistcontrol', 'cmd:add', 'track_id:1,3']]);
	expect(batchCommands([], 'add')).toEqual([]);
});

test('queued ids and the current track are read strictly and bounded', () => {
	expect(queuedIds([{ id: 1 }, { id: 1 }, { id: -3 }, { id: '4' }, {}, { id: 5 }])).toEqual([1, 5]);
	expect(queuedIds(Array.from({ length: 500 }, (_, i) => ({ id: i + 1 })))).toHaveLength(200);
	expect(currentTrack({ playlist_loop: [{ id: 9 }] })).toBe(9);
	expect(currentTrack({ playlist_loop: [{ id: -9 }] })).toBeNull();
	expect(currentTrack({})).toBeNull();
});

function harness(batches: NextBatch[]) {
	const requests: { station: number; req: NextRequest }[] = [];
	const sent: { player: string; cmd: unknown[] }[] = [];
	let release: (() => void) | null = null;
	let hold = false;
	const runner = new StationRunner({
		next: async (station, req) => {
			requests.push({ station, req });
			if (hold) await new Promise<void>((r) => (release = r));
			const b = batches.shift();
			if (!b) throw new Error('no more');
			return b;
		},
		rpc: async (player, cmd) => {
			sent.push({ player, cmd });
		},
		queue: async () => [11, 12],
	});
	return { runner, requests, sent, holdNext: (on: boolean) => (hold = on), release: () => release?.() };
}

test('a track station loads, then refills when low, leaving out what is queued', async () => {
	const h = harness([
		{ mode: 'tracks', tracks: [11, 12], refill: true },
		{ mode: 'tracks', tracks: [13], refill: true },
	]);
	await h.runner.start(7, P);
	expect(h.requests[0]).toEqual({ station: 7, req: { player: P, start: true, count: BATCH } });
	expect(h.sent).toEqual([{ player: P, cmd: ['playlistcontrol', 'cmd:load', 'track_id:11,12'] }]);
	expect(h.runner.active).toBe(true);
	// Plenty left: nothing.
	expect(h.runner.onStatus(P, { playlist_tracks: 20, playlist_cur_index: 0, playlist_loop: [{ id: 11 }] })).toBeNull();
	// Low: one refill.
	await h.runner.onStatus(P, { playlist_tracks: 2, playlist_cur_index: 0, playlist_loop: [{ id: 11 }] });
	expect(h.requests[1]!.req).toEqual({ player: P, start: false, count: BATCH, exclude: [11, 12] });
	expect(h.sent[1]).toEqual({ player: P, cmd: ['playlistcontrol', 'cmd:add', 'track_id:13'] });
});

test('never two refills at once', async () => {
	const h = harness([
		{ mode: 'tracks', tracks: [11], refill: true },
		{ mode: 'tracks', tracks: [12], refill: true },
	]);
	await h.runner.start(1, P);
	h.holdNext(true);
	const low = { playlist_tracks: 1, playlist_cur_index: 0, playlist_loop: [{ id: 11 }] };
	const first = h.runner.onStatus(P, low);
	expect(first).not.toBeNull();
	expect(h.runner.onStatus(P, low)).toBeNull();
	expect(h.runner.onStatus(P, low)).toBeNull();
	await new Promise((r) => setTimeout(r, 0));
	h.release();
	await first;
	expect(h.requests).toHaveLength(2);
});

test('it stops when the user plays something else or picks another player', async () => {
	const h = harness([{ mode: 'tracks', tracks: [11, 12], refill: true }]);
	await h.runner.start(1, P);
	expect(h.runner.onStatus(P, { playlist_tracks: 1, playlist_cur_index: 0, playlist_loop: [{ id: 99 }] })).toBeNull();
	expect(h.runner.active).toBe(false);
	const h2 = harness([{ mode: 'tracks', tracks: [11], refill: true }]);
	await h2.runner.start(1, P);
	expect(h2.runner.onStatus('02:4c:41:00:00:02', { playlist_tracks: 0 })).toBeNull();
	expect(h2.runner.active).toBe(false);
	expect(h2.requests).toHaveLength(1);
});

test('Random Mix is sent once; LMS keeps it filled', async () => {
	const h = harness([{ mode: 'randomplay', command: ['randomplay', 'tracks', 'genre_id:3'], refill: false }]);
	await h.runner.start(2, P);
	expect(h.sent).toEqual([{ player: P, cmd: ['randomplay', 'tracks', 'genre_id:3'] }]);
	expect(h.runner.onStatus(P, { playlist_tracks: 1, playlist_cur_index: 0 })).toBeNull();
	expect(h.requests).toHaveLength(1);
});

test('a menu station run by the gateway refills only when it can add', async () => {
	const h = harness([
		{ mode: 'ran', refill: true },
		{ mode: 'ran', refill: true },
	]);
	await h.runner.start(3, P);
	expect(h.sent).toEqual([]); // the gateway ran it
	await h.runner.onStatus(P, { playlist_tracks: 1, playlist_cur_index: 0, playlist_loop: [{ id: 50 }] });
	expect(h.requests[1]!.req.start).toBe(false);
	const h2 = harness([{ mode: 'ran', refill: false }]);
	await h2.runner.start(3, P);
	expect(h2.runner.onStatus(P, { playlist_tracks: 1, playlist_cur_index: 0 })).toBeNull();
	const h3 = harness([{ mode: 'none', refill: false }]);
	await h3.runner.start(3, P);
	expect(h3.runner.active).toBe(false);
});

test('an empty first batch says so; a failure stops the station', async () => {
	const h = harness([{ mode: 'tracks', tracks: [], refill: true }]);
	await h.runner.start(1, P);
	expect(h.runner.active).toBe(false);
	expect(h.runner.error).toMatch(/nothing new/);
	const h2 = harness([]);
	await expect(h2.runner.start(1, P)).rejects.toThrow();
	expect(h2.runner.active).toBe(false);
	const h3 = harness([{ mode: 'tracks', tracks: [11], refill: true }]);
	await h3.runner.start(1, P);
	await h3.runner.onStatus(P, { playlist_tracks: 1, playlist_cur_index: 0, playlist_loop: [{ id: 11 }] });
	expect(h3.runner.active).toBe(false);
	expect(h3.runner.error).toBe('no more');
});

test('an empty refill stops topping up', async () => {
	const h = harness([
		{ mode: 'tracks', tracks: [11], refill: true },
		{ mode: 'tracks', tracks: [], refill: true },
	]);
	await h.runner.start(1, P);
	const low = { playlist_tracks: 1, playlist_cur_index: 0, playlist_loop: [{ id: 11 }] };
	await h.runner.onStatus(P, low);
	expect(h.runner.onStatus(P, low)).toBeNull();
	expect(h.requests).toHaveLength(2);
});
