// Sleep timer rules (docs/plans: Wave 1 §1): how long until the end of the track, album or
// queue; when to set it again; which players; the fade-out setting; and the command itself.
// The same vectors are in mobile/src/sleep.rs (the Android app).
import * as c from '#lib/lms/commands';
import {
	DEFAULT_FADE_S,
	armedOver,
	choiceSeconds,
	customMinutes,
	fadeGain,
	formatLeft,
	groupOf,
	loadFade,
	rearmSeconds,
	remainingSeconds,
	sanitiseFade,
	saveFade,
	sleepLeft,
	sleepTargets,
	type Armed,
} from '#lib/lms/sleep';
import type { PlayerStatus, Track } from '#lib/lms/types';

const t = (album: number, duration?: number): Track => ({ id: album * 100 + (duration ?? 0), album_id: album, duration });
const status = (over: Partial<PlayerStatus> = {}): PlayerStatus => ({
	mode: 'play',
	time: 60,
	duration: 200,
	playlist_cur_index: 2,
	playlist_tracks: 6,
	playlist_loop: [{ id: 1, album_id: 7, duration: 200 }],
	...over,
});

describe('time left', () => {
	test('end of track: the rest of the current track, rounded up', () => {
		expect(remainingSeconds('track', status())).toBe(140);
		expect(remainingSeconds('track', status({ time: 59.2 }))).toBe(141);
		// Past the end (a late status): nothing left, not negative.
		expect(remainingSeconds('track', status({ time: 250 }))).toBe(0);
		// The track's own duration when the status has none.
		expect(remainingSeconds('track', status({ duration: undefined }))).toBe(140);
	});

	test('end of album: the run of tracks from the same album that follows', () => {
		const upcoming = [t(7, 180), t(7, 240), t(8, 300), t(7, 100)];
		expect(remainingSeconds('album', status(), upcoming)).toBe(140 + 180 + 240);
		// The next track is from another album: just this track.
		expect(remainingSeconds('album', status(), [t(9, 100)])).toBe(140);
		// No album id: as for the track.
		expect(remainingSeconds('album', status({ playlist_loop: [{ id: 1, duration: 200 }] }), upcoming)).toBe(140);
		// A track of the album without a length: unknowable.
		expect(remainingSeconds('album', status(), [t(7, 180), t(7)])).toBeNull();
	});

	test('end of queue: everything after the current track', () => {
		const upcoming = [t(7, 180), t(8, 240), t(9, 300)];
		expect(remainingSeconds('queue', status(), upcoming)).toBe(140 + 720);
		// The last track.
		expect(remainingSeconds('queue', status({ playlist_cur_index: 5 }), [])).toBe(140);
		// Only part of the queue read (a queue longer than one page): unknowable.
		expect(remainingSeconds('queue', status({ playlist_tracks: 900 }), upcoming)).toBeNull();
		// A radio stream in the queue.
		expect(remainingSeconds('queue', status(), [t(7, 180), { title: 'Radio' }, t(9, 300)])).toBeNull();
	});

	test('an end more than a day away is not offered', () => {
		const long = Array.from({ length: 30 }, () => t(8, 3600));
		expect(remainingSeconds('queue', status({ playlist_cur_index: 0, playlist_tracks: 31 }), long)).toBeNull();
		expect(remainingSeconds('queue', status({ playlist_cur_index: 0, playlist_tracks: 24 }), long.slice(0, 23))).toBe(140 + 23 * 3600);
		expect(remainingSeconds('track', status({ duration: 1e9 }))).toBeNull();
	});

	test('nothing playing, or a stream without a length: unknowable', () => {
		expect(remainingSeconds('track', null)).toBeNull();
		expect(remainingSeconds('track', status({ playlist_loop: [] }))).toBeNull();
		expect(remainingSeconds('track', status({ duration: 0, playlist_loop: [{ title: 'Radio' }] }))).toBeNull();
	});

	test('hostile numbers never make a negative, infinite or NaN timer', () => {
		const weird: unknown[] = [NaN, Infinity, -Infinity, -5, '1e308', '12abc', null, {}, [], '  ', 'Infinity'];
		for (const w of weird) {
			for (const end of ['track', 'album', 'queue'] as const) {
				const s = status({ time: w as number, duration: w as number, playlist_cur_index: w as number, playlist_tracks: w as number });
				const r = remainingSeconds(end, s, [{ duration: w as number, album_id: 7 }]);
				if (r !== null) {
					expect(Number.isFinite(r), `${String(w)} ${end}`).toBe(true);
					expect(r).toBeGreaterThanOrEqual(0);
				}
			}
		}
	});

	test('fixed lengths and custom lengths', () => {
		expect(choiceSeconds({ kind: 'minutes', minutes: 30 }, null)).toBe(1800);
		expect(choiceSeconds({ kind: 'minutes', minutes: 0 }, null)).toBeNull();
		expect(choiceSeconds({ kind: 'minutes', minutes: 1441 }, null)).toBeNull();
		expect(choiceSeconds({ kind: 'track' }, status())).toBe(140);
		expect(customMinutes('45')).toBe(45);
		expect(customMinutes(' 90 ')).toBe(90);
		for (const bad of ['', '0', '-5', '1.5', '1e3', '1441', 'abc', '45;sleep 0', null, undefined, NaN, Infinity, 2.5]) {
			expect(customMinutes(bad), String(bad)).toBeNull();
		}
	});
});

describe('keeping an end-of-queue timer in step', () => {
	const armed: Armed = { end: 'track', players: ['aa:bb:cc:dd:ee:01'], endsAt: 1_000_000 + 140_000, setAt: 1_000_000 };

	test('unchanged (within the tolerance): left alone', () => {
		expect(rearmSeconds(armed, 140, 1_000_000)).toBeNull();
		expect(rearmSeconds(armed, 136, 1_002_000)).toBeNull(); // the poll ran a little late
	});

	test('a seek, skip, pause or queue edit moves the end: set again', () => {
		expect(rearmSeconds(armed, 40, 1_000_000)).toBe(40); // seeked forward
		expect(rearmSeconds(armed, 140, 1_030_000)).toBe(140); // paused for 30 s
		expect(rearmSeconds(armed, 0.2, 1_000_000)).toBe(1); // never 0 (which would cancel)
	});

	test('nothing known: left alone', () => {
		expect(rearmSeconds(armed, null, 1_000_000)).toBeNull();
		expect(rearmSeconds(armed, NaN, 1_000_000)).toBeNull();
	});

	test('over once LMS stops reporting it (it went off, or was cancelled elsewhere)', () => {
		expect(armedOver(armed, 120, 1_020_000)).toBe(false);
		// Just set: LMS may not report it yet.
		expect(armedOver(armed, undefined, 1_003_000)).toBe(false);
		expect(armedOver(armed, undefined, 1_011_000)).toBe(true);
		expect(armedOver(armed, '0', 1_011_000)).toBe(true);
	});

	test('what LMS reports', () => {
		expect(sleepLeft(status({ will_sleep_in: 1799.4 }))).toBe(1800);
		expect(sleepLeft(status({ will_sleep_in: '60' }))).toBe(60);
		for (const none of [undefined, 0, '0', -3, 'x', NaN]) expect(sleepLeft(status({ will_sleep_in: none as number }))).toBeNull();
		expect(sleepLeft(null)).toBeNull();
	});
});

describe('which players', () => {
	const ME = 'aa:bb:cc:dd:ee:01';
	const players = [
		{ playerid: ME, name: 'Mine' },
		{ playerid: 'aa:bb:cc:dd:ee:02', name: 'Kitchen', connected: 1 },
		{ playerid: 'aa:bb:cc:dd:ee:03', name: 'Gone', connected: 0 },
	];

	test('this player, its sync group, or every connected player listed', () => {
		const s = status({ sync_master: ME, sync_slaves: 'AA:BB:CC:DD:EE:02,aa:bb:cc:dd:ee:04' });
		expect(sleepTargets('player', ME, s, players)).toEqual([ME]);
		expect(sleepTargets('group', ME, s, players)).toEqual([ME, 'aa:bb:cc:dd:ee:02', 'aa:bb:cc:dd:ee:04']);
		expect(sleepTargets('all', ME, s, players)).toEqual([ME, 'aa:bb:cc:dd:ee:02']);
		expect(sleepTargets('group', ME, status(), players)).toEqual([ME]); // not grouped
		expect(sleepTargets('all', '', s, players)).toEqual([]);
	});

	test('junk in the sync fields never becomes a target', () => {
		const s = status({ sync_master: '../x', sync_slaves: 'all, ,aa:bb:cc:dd:ee,aa:bb:cc:dd:ee:05;rescan,aa:bb:cc:dd:ee:06' });
		expect(groupOf(ME, s)).toEqual([ME, 'aa:bb:cc:dd:ee:06']);
	});
});

describe('fade-out', () => {
	test('saved per user, checked, 30 s by default', () => {
		const m = new Map<string, string>();
		const store = { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
		expect(loadFade('alice', store)).toBe(DEFAULT_FADE_S);
		saveFade('alice', 90, store);
		saveFade('bob', 0, store);
		expect(loadFade('alice', store)).toBe(90);
		expect(loadFade('bob', store)).toBe(0);
		// Out of range or junk: the default.
		saveFade('carol', 9999, store);
		expect(loadFade('carol', store)).toBe(DEFAULT_FADE_S);
		for (const bad of ['', '-1', '121', '1e2', 'x', '30;x', null, 30.5]) expect(sanitiseFade(bad), String(bad)).toBe(DEFAULT_FADE_S);
		// A hostile user name stays within its own key.
		saveFade('../../anywhere.player', 10, store);
		expect([...m.keys()].every((k) => k.startsWith('anywhere.sleep.fade.') && !k.includes('/'))).toBe(true);
		// Storage that throws: the default, no crash.
		const broken = { getItem: () => { throw new Error('blocked'); }, setItem: () => { throw new Error('blocked'); } };
		expect(loadFade('alice', broken)).toBe(DEFAULT_FADE_S);
		expect(() => saveFade('alice', 10, broken)).not.toThrow();
	});

	test('full volume until the fade starts, silence at the end', () => {
		expect(fadeGain(null, 30)).toBe(1);
		expect(fadeGain(100, 30)).toBe(1);
		expect(fadeGain(30, 30)).toBe(1);
		expect(fadeGain(15, 30)).toBeCloseTo(0.25);
		expect(fadeGain(0, 30)).toBe(0);
		expect(fadeGain(-4, 30)).toBe(0);
		expect(fadeGain(5, 0)).toBe(1); // fade off
		for (let s = 30; s > 0; s -= 0.5) expect(fadeGain(s - 0.5, 30)).toBeLessThanOrEqual(fadeGain(s, 30)); // only ever down
		expect(fadeGain(NaN, 30)).toBe(1);
	});

	test('time left in words', () => {
		expect(formatLeft(45)).toBe('45 s');
		expect(formatLeft(1620)).toBe('27 min');
		expect(formatLeft(3900)).toBe('1 h 5 min');
		expect(formatLeft(7200)).toBe('2 h');
		expect(formatLeft(-3)).toBe('0 s');
	});
});

describe('the command', () => {
	test('whole seconds within a day; 0 cancels', () => {
		expect(c.sleep(1800)).toEqual(['sleep', 1800]);
		expect(c.sleep(139.2)).toEqual(['sleep', 140]);
		expect(c.sleep(0)).toEqual(['sleep', 0]);
		expect(c.sleep(-50)).toEqual(['sleep', 0]);
		expect(c.sleep(1e12)).toEqual(['sleep', c.MAX_SLEEP_SECONDS]);
		for (const bad of [NaN, Infinity, -Infinity]) expect(c.sleep(bad)).toEqual(['sleep', 0]);
		// Only digits ever reach the gateway (its allow-list takes `sleep <uint>` or `sleep ?`).
		for (const v of [1800, 1e12, -1, NaN, 0.5]) expect(String(c.sleep(v)[1])).toMatch(/^\d+$/);
	});

	test('status asks for the current and the next track; queue times only durations and albums', () => {
		expect(c.status()).toEqual(['status', '-', 2, c.STATUS_TAGS]);
		expect(c.queueTimes(-3)).toEqual(['status', 0, 500, 'tags:de']);
	});
});
