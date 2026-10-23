// What works while the server is out of reach (SPEC: controls while the server is out of
// reach): the gating table, Next/Previous/end-of-track over what this device has, queue
// marks, and what to tell LMS when it is back.
import { MARK_LABELS, NEXT_NEEDS_SERVER, afterEnd, gates, nextIndex, previousTarget, queueMark, reconcile, type GateInput, type LocalOutcome } from '#lib/player/local-plan';

const set = (...xs: number[]) => (i: number) => xs.includes(i);
const q = (index: number, count = 5, repeat = 0) => ({ index, count, repeat });

const base: GateInput = {
	reach: 'unreachable',
	hasNow: true,
	thisDevice: true,
	queue: q(2),
	elapsed: 10,
	available: set(2, 3),
	fromStart: set(3),
	source: 'stream',
	streamControllable: true,
};

describe('gating', () => {
	test('online: everything, as before (only a status is needed)', () => {
		const g = gates({ ...base, reach: 'online', thisDevice: false, available: () => false });
		expect(g).toMatchObject({ playPause: true, next: true, previous: true, seek: true, volume: true, other: true });
		expect(g.jump(4)).toBe(true);
		const none = gates({ ...base, reach: 'online', hasNow: false });
		expect(none).toMatchObject({ playPause: false, next: false, previous: false, seek: false, volume: false, other: false });
	});

	test('a remote player out of reach (or busy): every control disabled', () => {
		for (const reach of ['unreachable', 'busy'] as const) {
			const g = gates({ ...base, reach, thisDevice: false });
			expect(g, reach).toMatchObject({ playPause: false, next: false, previous: false, seek: false, volume: false, other: false });
			for (let i = 0; i < 5; i++) expect(g.jump(i)).toBe(false);
		}
	});

	test('this device: play/pause works on its stream; volume and everything else needs the server', () => {
		const g = gates(base);
		expect(g.playPause).toBe(true);
		expect(g.volume).toBe(false);
		expect(g.other).toBe(false);
		expect(g.seek).toBe(false); // a stream can't be sought without LMS
		expect(gates({ ...base, streamControllable: false }).playPause).toBe(false);
	});

	test('busy gates like out of reach', () => {
		expect(gates({ ...base, reach: 'busy' })).toMatchObject({ playPause: true, next: true, other: false, volume: false });
	});

	test('a download playing: play/pause and seeking work', () => {
		const g = gates({ ...base, source: 'download', queue: q(3) });
		expect(g.playPause).toBe(true);
		expect(g.seek).toBe(true);
	});

	test('nothing playing: play works only if the current track is downloaded', () => {
		expect(gates({ ...base, source: null, queue: q(3) }).playPause).toBe(true);
		expect(gates({ ...base, source: null, queue: q(2) }).playPause).toBe(false);
	});

	test('next/previous/jump follow what is on this device', () => {
		const g = gates(base);
		expect(g.next).toBe(true); // 3 is downloaded
		expect(g.previous).toBe(false); // 1 isn't, and the stream can't restart
		expect([0, 1, 2, 3, 4].map((i) => g.jump(i))).toEqual([false, false, true, true, false]);
		expect(g.jump(-1)).toBe(false);
		expect(g.jump(99)).toBe(false);
		expect(gates({ ...base, queue: q(3), available: set(2, 3), fromStart: set(3) }).next).toBe(false);
	});
});

describe('next and previous', () => {
	test('next: only the entry after, if it is here; repeat-all wraps', () => {
		expect(nextIndex(q(1), set(2))).toBe(2);
		expect(nextIndex(q(1), set(3))).toBeNull(); // never skips over one that isn't here
		expect(nextIndex(q(4), set(0))).toBeNull();
		expect(nextIndex(q(4, 5, 2), set(0))).toBe(0);
		expect(nextIndex(q(0, 1, 2), set(0))).toBeNull(); // a queue of one doesn't "wrap" to itself
		expect(nextIndex(q(0, 0), () => true)).toBeNull();
		expect(nextIndex(q(-1), () => true)).toBeNull();
	});

	test('previous: restart after a few seconds (only from a download), else the entry before', () => {
		expect(previousTarget(q(2), 10, set(1), set(2))).toEqual({ restart: true });
		expect(previousTarget(q(2), 10, set(1), set())).toEqual({ index: 1 });
		expect(previousTarget(q(2), 2, set(1), set(2))).toEqual({ index: 1 });
		expect(previousTarget(q(2), 2, set(), set(2))).toBeNull();
		expect(previousTarget(q(0), 1, set(4), set())).toBeNull();
		expect(previousTarget(q(0, 5, 2), 1, set(4), set())).toEqual({ index: 4 });
	});

	test('the end of a track: repeat one replays a download, else on to the next one here', () => {
		expect(afterEnd(q(2, 5, 1), set(3), set(2))).toBe(2);
		expect(afterEnd(q(2, 5, 1), set(3), set())).toBe(3);
		expect(afterEnd(q(2), set(3), set(3))).toBe(3);
		expect(afterEnd(q(2), set(), set())).toBeNull();
		expect(NEXT_NEEDS_SERVER).toBe('Next track needs your music server.');
	});
});

describe('queue marks', () => {
	const downloaded = (id: unknown) => id === 7;
	test('downloads for any player; the held stream only while this device is the player', () => {
		expect(queueMark({ id: 7, index: 0, downloaded, thisDevice: false, heldIndex: -1 })).toBe('downloaded');
		expect(queueMark({ id: 8, index: 2, downloaded, thisDevice: true, heldIndex: 2 })).toBe('device');
		expect(queueMark({ id: 8, index: 2, downloaded, thisDevice: false, heldIndex: 2 })).toBeNull();
		expect(queueMark({ id: 8, index: 3, downloaded, thisDevice: true, heldIndex: 2 })).toBeNull();
		expect(queueMark({ id: 8, index: 0, downloaded, thisDevice: true, heldIndex: -1 })).toBeNull();
		expect(MARK_LABELS).toEqual({ downloaded: 'Downloaded', device: 'On this device' });
	});
});

describe('reconciliation', () => {
	const local = (o: Partial<LocalOutcome> = {}): LocalOutcome => ({ index: 3, track: 33, time: 42.6, paused: false, sameStream: false, ...o });
	const lms = { mode: 'play', playlist_cur_index: 2, playlist_tracks: 5 };

	test('the same track still at that place: tell LMS where this device is', () => {
		expect(reconcile(local(), lms, 33)).toEqual({ follow: false, commands: [['playlist', 'index', 3], ['time', 42]], release: 'flush' });
		expect(reconcile(local({ paused: true }), lms, '33')).toEqual({
			follow: false,
			commands: [['playlist', 'index', 3], ['time', 42], ['pause', 1]],
			release: 'paused',
		});
		// Near the start: no seek.
		expect(reconcile(local({ time: 1.5 }), lms, 33)).toMatchObject({ commands: [['playlist', 'index', 3]] });
	});

	test('the queue changed meanwhile: follow LMS (nothing sent)', () => {
		expect(reconcile(local(), lms, 34)).toEqual({ follow: true, commands: [] });
		expect(reconcile(local(), lms, undefined)).toEqual({ follow: true, commands: [] });
		expect(reconcile(local({ index: 5 }), lms, 33)).toEqual({ follow: true, commands: [] }); // past the end now
		expect(reconcile(local({ track: undefined }), lms, undefined)).toEqual({ follow: true, commands: [] });
		expect(reconcile(local(), null, 33)).toEqual({ follow: true, commands: [] });
		expect(reconcile(local({ index: -1 }), lms, 33)).toEqual({ follow: true, commands: [] });
	});

	test('untrusted ids never match by accident', () => {
		for (const bad of [{}, [], '33 ', 'x'.repeat(200), '33;drop', null, NaN]) expect(reconcile(local(), lms, bad).follow, String(bad)).toBe(true);
		expect(reconcile(local({ track: '../x' }), lms, '../x').follow).toBe(true);
	});

	test('still on the same stream, same connection: only the pause state is told', () => {
		const same = local({ index: 2, track: 22, sameStream: true });
		expect(reconcile(same, lms, 22)).toEqual({ follow: false, commands: [], release: 'now' });
		expect(reconcile({ ...same, paused: true }, lms, 22)).toEqual({ follow: false, commands: [['pause', 1]], release: 'paused' });
		expect(reconcile(same, { ...lms, mode: 'pause' }, 22)).toEqual({ follow: false, commands: [['pause', 0]], release: 'now' });
		// LMS stopped meanwhile (the stream ended): put it back where this device is.
		expect(reconcile(same, { ...lms, mode: 'stop' }, 22).commands[0]).toEqual(['playlist', 'index', 2]);
	});
});
