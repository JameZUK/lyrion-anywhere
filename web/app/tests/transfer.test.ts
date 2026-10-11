// Transfer (docs/PROTOCOL.md): move playback from one player to another. The plans are pure;
// the runner is driven against a scripted LMS.
import type { Command } from '$lib/api/rpc';
import { chunkIds, ID_LIST_MAX, loadTracks } from '$lib/lms/commands';
import { MAX_COPY, grouped, planCopyTransfer, planGroupTransfer, refusalText, transfer, type Step } from '$lib/lms/transfer';

const SRC = '02:4c:41:00:00:01';
const DST = 'aa:bb:cc:dd:ee:02';

const cmds = (steps: Step[]) => steps.map((s) => ('verify' in s ? 'verify' : `${s.on}: ${s.cmd.join(' ')}`));

describe('recipe 1: group, then leave', () => {
	test('playing: power on, join, check, leave, stop; the target is already playing', () => {
		const p = planGroupTransfer(SRC, DST, { mode: 'play', playlist_tracks: 6, time: 31.5 });
		expect(p.ok).toBe(true);
		if (!p.ok) return;
		expect(cmds(p.steps)).toEqual([
			`target: power 1`,
			`source: sync ${DST}`,
			'verify',
			'source: sync -',
			'source: stop',
		]);
	});

	test('paused: the target is unpaused; stopped: it is started', () => {
		const paused = planGroupTransfer(SRC, DST, { mode: 'pause', playlist_tracks: '3' });
		const stopped = planGroupTransfer(SRC, DST, { mode: 'stop', playlist_tracks: 3 });
		expect(paused.ok && cmds(paused.steps).at(-1)).toBe('target: pause 0');
		expect(stopped.ok && cmds(stopped.steps).at(-1)).toBe('target: play');
	});

	test('radio and app items move too (LMS copies the whole queue)', () => {
		const p = planGroupTransfer(SRC, DST, { mode: 'play', playlist_tracks: 1 });
		expect(p.ok).toBe(true);
	});

	test('an empty queue, no status, or the same player: refused', () => {
		expect(planGroupTransfer(SRC, DST, { mode: 'stop', playlist_tracks: 0 })).toEqual({ ok: false, refusal: 'empty' });
		expect(planGroupTransfer(SRC, DST, {})).toEqual({ ok: false, refusal: 'empty' });
		expect(planGroupTransfer(SRC, DST, null)).toEqual({ ok: false, refusal: 'empty' });
		expect(planGroupTransfer(SRC, SRC.toUpperCase(), { playlist_tracks: 4 })).toEqual({ ok: false, refusal: 'same' });
	});

	test('ids that are not player ids are refused before anything is planned', () => {
		for (const bad of ['', '-', '?', 'x; rescan', '02:4c:41:00:00:01 stop', '../02:4c']) {
			expect(() => planGroupTransfer(SRC, bad, { playlist_tracks: 1 }), bad).toThrow(RangeError);
			expect(() => planGroupTransfer(bad, DST, { playlist_tracks: 1 }), bad).toThrow(RangeError);
		}
	});
});

describe('recipe 2: copy the list', () => {
	const queue = (ids: unknown[]) => ids.map((id) => ({ id }));

	test('loads the same tracks in play order at the same track and time, then stops the source', () => {
		const p = planCopyTransfer(SRC, DST, { mode: 'play', playlist_tracks: 3, playlist_cur_index: '1', time: 83.7, 'playlist repeat': 2 }, queue([11, 12, 13]));
		expect(p.ok && cmds(p.steps)).toEqual([
			'target: power 1',
			'target: playlist shuffle 0',
			'target: playlist repeat 2',
			'target: playlistcontrol cmd:load track_id:11,12,13 play_index:1',
			'target: time 83',
			'source: stop',
		]);
	});

	test('near the start of a track: no seek', () => {
		const p = planCopyTransfer(SRC, DST, { mode: 'pause', playlist_tracks: 1, playlist_cur_index: 0, time: 1.2 }, queue([5]));
		expect(p.ok && cmds(p.steps).some((c) => c.includes('time'))).toBe(false);
	});

	test('long queues go in lists the gateway accepts, and the jump comes after them', () => {
		const ids = Array.from({ length: 120 }, (_, i) => 2_000_000 + i);
		const p = planCopyTransfer(SRC, DST, { mode: 'play', playlist_tracks: 120, playlist_cur_index: 100, time: 0 }, queue(ids));
		expect(p.ok).toBe(true);
		if (!p.ok) return;
		const loads = p.steps.filter((s): s is { on: 'target'; cmd: Command } => 'cmd' in s && s.cmd[0] === 'playlistcontrol');
		expect(loads[0]!.cmd[1]).toBe('cmd:load');
		expect(loads.slice(1).every((s) => s.cmd[1] === 'cmd:add')).toBe(true);
		for (const s of loads) expect(String(s.cmd[2]).length - 'track_id:'.length).toBeLessThanOrEqual(ID_LIST_MAX);
		// Every id, once, in order.
		expect(loads.flatMap((s) => String(s.cmd[2]).slice(9).split(',').map(Number))).toEqual(ids);
		// Index 100 is not in the first list: start that list at 0, then jump.
		expect(loads[0]!.cmd[3]).toBe('play_index:0');
		expect(cmds(p.steps)).toContain('target: playlist index 100');
	});

	test('radio, app items and anything without a library id: refused', () => {
		const s = { mode: 'play', playlist_tracks: 2, playlist_cur_index: 0 };
		expect(planCopyTransfer(SRC, DST, s, queue([1, '-108699300821096']))).toEqual({ ok: false, refusal: 'remote' });
		expect(planCopyTransfer(SRC, DST, s, [{ id: 1 }, { id: 2, remote: 1 }])).toEqual({ ok: false, refusal: 'remote' });
		expect(planCopyTransfer(SRC, DST, s, queue([1, undefined]))).toEqual({ ok: false, refusal: 'remote' });
		expect(planCopyTransfer(SRC, DST, s, queue([1, 'file:///etc/passwd']))).toEqual({ ok: false, refusal: 'remote' });
		expect(planCopyTransfer(SRC, DST, s, queue([1, '2,3']))).toEqual({ ok: false, refusal: 'remote' });
	});

	test('empty, too long, or a queue that did not all arrive: refused', () => {
		expect(planCopyTransfer(SRC, DST, { playlist_tracks: 0 }, [])).toEqual({ ok: false, refusal: 'empty' });
		expect(planCopyTransfer(SRC, DST, null, [])).toEqual({ ok: false, refusal: 'empty' });
		const many = Array.from({ length: MAX_COPY + 1 }, (_, i) => ({ id: i + 1 }));
		expect(planCopyTransfer(SRC, DST, { playlist_tracks: MAX_COPY + 1 }, many)).toEqual({ ok: false, refusal: 'too-long' });
		expect(planCopyTransfer(SRC, DST, { playlist_tracks: 5 }, queue([1, 2]))).toEqual({ ok: false, refusal: 'too-long' });
	});

	test('a hostile status never reaches a command unchecked', () => {
		const p = planCopyTransfer(SRC, DST, { playlist_tracks: 2, playlist_cur_index: 'x; stop', time: '1e9; rescan', 'playlist repeat': 9 }, queue([1, 2]));
		expect(p.ok).toBe(true);
		if (!p.ok) return;
		for (const s of p.steps) if ('cmd' in s) for (const a of s.cmd) expect(String(a)).not.toMatch(/[;\s]/);
		expect(cmds(p.steps)).toContain('target: playlist repeat 0');
	});
});

test('chunkIds keeps order and limits, and checks every id', () => {
	expect(chunkIds([1, 22, 333], 6)).toEqual([[1, 22], [333]]);
	expect(chunkIds([])).toEqual([]);
	expect(() => chunkIds([1, -2])).toThrow(RangeError);
	expect(() => loadTracks([], 'load')).toThrow(RangeError);
	expect(() => loadTracks(Array.from({ length: 40 }, () => 1234567), 'add')).toThrow(RangeError);
	expect(loadTracks([7, 8], 'add', 1)).toEqual(['playlistcontrol', 'cmd:add', 'track_id:7,8']);
});

test('grouped: the target lists the source, with the same number of tracks', () => {
	expect(grouped(SRC, { sync_master: SRC, sync_slaves: DST, playlist_tracks: 6 }, 6)).toBe(true);
	expect(grouped(SRC, { sync_master: 'aa:bb:cc:dd:ee:09', sync_slaves: `${DST},${SRC.toUpperCase()}`, playlist_tracks: 6 }, 6)).toBe(true);
	expect(grouped(SRC, { sync_master: SRC, playlist_tracks: 5 }, 6)).toBe(false);
	expect(grouped(SRC, { playlist_tracks: 6 }, 6)).toBe(false);
	expect(grouped(SRC, null, 6)).toBe(false);
});

test('refusals read as sentences that name the player', () => {
	expect(refusalText('remote', 'Study')).toContain('Study');
	expect(refusalText('empty', 'Study')).toMatch(/queue is empty/);
	expect(refusalText('too-long', 'Study')).toContain(String(MAX_COPY));
});

// --- the runner, against a scripted LMS ------------------------------------------------------

type Call = { player: string; cmd: Command };
function fakeLms(o: { groups?: boolean; queue?: unknown[]; fail?: string; mode?: string } = {}) {
	const calls: Call[] = [];
	let synced = false;
	const ids = o.queue ?? [11, 12, 13];
	const rpc = async <T,>(player: string, cmd: Command): Promise<T> => {
		calls.push({ player, cmd });
		const c = cmd.join(' ');
		if (o.fail && c.startsWith(o.fail)) throw new Error('LMS unavailable');
		if (c === `sync ${DST}` && player === SRC && o.groups !== false) synced = true;
		if (c === 'sync -') synced = false;
		if (cmd[0] === 'status' && cmd[1] === '-') {
			if (player === SRC) return { mode: o.mode ?? 'play', time: 40, playlist_tracks: ids.length, playlist_cur_index: 1 } as T;
			return (synced ? { sync_master: SRC, sync_slaves: DST, playlist_tracks: ids.length } : { playlist_tracks: 0 }) as T;
		}
		if (cmd[0] === 'status') return { playlist_loop: ids.map((id) => ({ id })) } as T;
		return {} as T;
	};
	return { rpc, calls, sent: () => calls.filter((c) => c.cmd[0] !== 'status').map((c) => `${c.player === SRC ? 'source' : 'target'}: ${c.cmd.join(' ')}`) };
}
const quick = { sleep: async () => {} };

test('runner: grouping works, so the queue moves with LMS doing the copying', async () => {
	const lms = fakeLms();
	expect(await transfer(lms.rpc, SRC, DST, quick)).toEqual({ ok: true, method: 'group' });
	expect(lms.sent()).toEqual(['target: power 1', `source: sync ${DST}`, 'source: sync -', 'source: stop']);
});

test("runner: LMS won't group them: the target is taken out again and the list copied", async () => {
	const lms = fakeLms({ groups: false });
	expect(await transfer(lms.rpc, SRC, DST, quick)).toEqual({ ok: true, method: 'copy' });
	const sent = lms.sent();
	expect(sent.slice(0, 3)).toEqual(['target: power 1', `source: sync ${DST}`, 'target: sync -']);
	expect(sent).toContain('target: playlistcontrol cmd:load track_id:11,12,13 play_index:1');
	expect(sent.at(-1)).toBe('source: stop');
});

test("runner: can't group, and the queue has radio in it: refused, the source left playing", async () => {
	const lms = fakeLms({ groups: false, queue: [11, '-1086993'] });
	expect(await transfer(lms.rpc, SRC, DST, quick)).toEqual({ ok: false, refusal: 'remote' });
	expect(lms.sent()).not.toContain('source: stop');
});

test('runner: an empty queue sends nothing at all', async () => {
	const lms = fakeLms({ queue: [] });
	expect(await transfer(lms.rpc, SRC, DST, quick)).toEqual({ ok: false, refusal: 'empty' });
	expect(lms.sent()).toEqual([]);
});

test('runner: a failure after joining never leaves the two grouped', async () => {
	const lms = fakeLms({ fail: 'sync -' });
	const r = await transfer(lms.rpc, SRC, DST, quick);
	expect(r).toEqual({ ok: false, error: 'LMS unavailable' });
	expect(lms.calls.at(-1)).toEqual({ player: DST, cmd: ['sync', '-'] });
});

test('runner: a refused command (e.g. a player this user may not control) is an error, not a crash', async () => {
	const lms = fakeLms({ fail: 'power' });
	expect(await transfer(lms.rpc, SRC, DST, quick)).toEqual({ ok: false, error: 'LMS unavailable' });
	expect(lms.sent()).toEqual(['target: power 1']);
});
