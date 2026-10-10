// Resume after a reload: the saved point is validated, and playback goes back only when needed.
import { MAX_AGE_MS, clearResume, loadResume, planResume, resumeCommands, saveResume, type ResumePoint } from '$lib/player/resume';
import type { Command } from '$lib/api/rpc';

const mem = () => {
	const m = new Map<string, string>();
	return { m, getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v), removeItem: (k: string) => void m.delete(k) };
};
const point: ResumePoint = { mac: '02:4c:41:00:00:01', playing: true, index: 3, time: 95.4, at: 1_000_000 };

test('a saved point comes back while recent, and not after', () => {
	const s = mem();
	saveResume(point, s);
	expect(loadResume(s, point.at + 1000)).toEqual(point);
	expect(loadResume(s, point.at + MAX_AGE_MS + 1)).toBeNull();
	expect(loadResume(s, point.at - 120_000)).toBeNull(); // from the future: clock jump
	clearResume(s);
	expect(loadResume(s, point.at)).toBeNull();
});

test('malformed or hostile saved points are ignored', () => {
	const s = mem();
	for (const bad of [
		'not json',
		'null',
		'[]',
		JSON.stringify({ ...point, mac: 'evil' }),
		JSON.stringify({ ...point, mac: '02:4c:41:00:00:01; rm' }),
		JSON.stringify({ ...point, index: -1 }),
		JSON.stringify({ ...point, index: 'x' }),
		JSON.stringify({ ...point, time: Number.MAX_VALUE * 10 }),
		JSON.stringify({ ...point, at: undefined }),
	]) {
		s.m.set('anywhere.device.resume', bad);
		expect(loadResume(s, point.at)).toBeNull();
	}
	s.m.set('anywhere.device.resume', JSON.stringify({ ...point, playing: 'yes', index: 2.7 }));
	expect(loadResume(s, point.at)).toMatchObject({ playing: false, index: 2 });
	expect(loadResume(undefined)).toBeNull();
	const throwing = { getItem: () => { throw new Error('blocked'); }, setItem: () => { throw new Error('blocked'); }, removeItem: () => { throw new Error('blocked'); } };
	expect(loadResume(throwing)).toBeNull();
	saveResume(point, throwing);
	clearResume(throwing);
});

test('playback goes back to the saved track and time only if LMS has not carried on', () => {
	const stopped = { mode: 'stop', playlist_cur_index: '0', playlist_tracks: 10 };
	expect(resumeCommands(point, stopped)).toEqual([
		['playlist', 'index', 3],
		['time', 95],
	]);
	expect(resumeCommands({ ...point, time: 1 }, stopped)).toEqual([['playlist', 'index', 3]]);
	expect(resumeCommands(point, { ...stopped, mode: 'play' })).toEqual([]);
	expect(resumeCommands({ ...point, playing: false }, stopped)).toEqual([]);
	expect(resumeCommands(point, { ...stopped, playlist_tracks: 3 })).toEqual([]); // queue changed
	expect(resumeCommands(point, null)).toEqual([]);
});

test('the saved track id round-trips, and a hostile one is refused', () => {
	const s = mem();
	saveResume({ ...point, track: 1234 }, s);
	expect(loadResume(s, point.at)).toEqual({ ...point, track: 1234 });
	saveResume({ ...point, track: -98765 }, s); // remote tracks have negative ids
	expect(loadResume(s, point.at)?.track).toBe(-98765);
	for (const bad of [1.5, '', 'x'.repeat(500), '<script>', { id: 1 }, null]) {
		s.m.set('anywhere.device.resume', JSON.stringify({ ...point, track: bad }));
		expect(loadResume(s, point.at), String(bad)).toBeNull();
	}
});

// Regression: a changed queue made the resume jump to whatever track was now at that index.
test('a resume point with a track only jumps if that same track is still at the index', () => {
	const stopped = { mode: 'stop', playlist_cur_index: '0', playlist_tracks: 10 };
	const p = { ...point, track: 77 };
	expect(resumeCommands(p, stopped, 77)).toEqual([
		['playlist', 'index', 3],
		['time', 95],
	]);
	expect(resumeCommands(p, stopped, '77')).toHaveLength(2); // LMS may give ids as strings
	expect(resumeCommands(p, stopped, 78)).toEqual([]);
	expect(resumeCommands(p, stopped, undefined)).toEqual([]);
});

describe('planResume', () => {
	const mac = point.mac;
	function fakeRpc(byPlayer: Record<string, { status: object; at?: object }>) {
		const calls: { player: string; cmd: Command }[] = [];
		const rpc = async <T,>(player: string, cmd: Command): Promise<T> => {
			calls.push({ player, cmd });
			const r = byPlayer[player];
			if (!r) throw new Error('unknown player');
			return (cmd[1] === '-' ? r.status : (r.at ?? {})) as T;
		};
		return { rpc, calls };
	}

	// Regression: the status checked was the selected player's (app.now), which may be a
	// different player from this device.
	test('asks about this device’s own player, never the selected one', async () => {
		const { rpc, calls } = fakeRpc({
			[mac]: { status: { mode: 'stop', playlist_tracks: 10 }, at: { playlist_loop: [{ id: 77 }] } },
			'aa:bb:cc:dd:ee:01': { status: { mode: 'play', playlist_tracks: 50 } },
		});
		expect(await planResume({ ...point, track: 77 }, rpc)).toEqual([
			['playlist', 'index', 3],
			['time', 95],
		]);
		expect(calls.every((c) => c.player === mac)).toBe(true);
		expect(calls[0]!.cmd[0]).toBe('status');
		expect(calls[1]!.cmd.slice(0, 3)).toEqual(['status', 3, 1]); // the track at the saved index
	});

	test('a different track at the saved index means no jump', async () => {
		const { rpc } = fakeRpc({ [mac]: { status: { mode: 'stop', playlist_tracks: 10 }, at: { playlist_loop: [{ id: 5 }] } } });
		expect(await planResume({ ...point, track: 77 }, rpc)).toEqual([]);
	});

	test('nothing to do (already playing) asks nothing more', async () => {
		const { rpc, calls } = fakeRpc({ [mac]: { status: { mode: 'play', playlist_tracks: 10 } } });
		expect(await planResume({ ...point, track: 77 }, rpc)).toEqual([]);
		expect(calls).toHaveLength(1);
	});

	test('an older point without a track id still resumes by index', async () => {
		const { rpc, calls } = fakeRpc({ [mac]: { status: { mode: 'stop', playlist_tracks: 10 } } });
		expect(await planResume(point, rpc)).toHaveLength(2);
		expect(calls).toHaveLength(1);
	});
});
