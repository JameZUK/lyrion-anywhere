// Clicking the position bar of a track that isn't playing starts it there.
import { seekAndPlay } from '#lib/lms/commands';

describe('seekAndPlay', () => {
	it('only seeks while playing', () => {
		expect(seekAndPlay('play', 61.24)).toEqual([['time', 61.2]]);
	});

	it('seeks, then resumes, when paused', () => {
		expect(seekAndPlay('pause', 200)).toEqual([['time', 200], ['pause', 0]]);
	});

	it('starts, then seeks, when stopped or unknown', () => {
		for (const mode of ['stop', undefined, null, '', 'odd', 3]) {
			expect(seekAndPlay(mode, 30)).toEqual([['play'], ['time', 30]]);
		}
	});

	it('never seeks before the start', () => {
		expect(seekAndPlay('pause', -5)[0]).toEqual(['time', 0]);
	});
});
