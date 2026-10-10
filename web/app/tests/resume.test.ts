// Resume after a reload: the saved point is validated, and playback goes back only when needed.
import { MAX_AGE_MS, clearResume, loadResume, resumeCommands, saveResume, type ResumePoint } from '$lib/player/resume';

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
