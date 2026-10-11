// Display and flag helpers for LMS values.
import { isRemote } from '$lib/lms/format';

describe('isRemote', () => {
	it('treats LMS text "0" as a local track (regression: the waveform never loaded)', () => {
		// Real LMS player status sends `"remote":"0"` for library tracks.
		expect(isRemote('0')).toBe(false);
		expect(isRemote(' 0 ')).toBe(false);
	});

	it('reads numbers, text and booleans', () => {
		expect(isRemote(0)).toBe(false);
		expect(isRemote(1)).toBe(true);
		expect(isRemote('1')).toBe(true);
		expect(isRemote(true)).toBe(true);
		expect(isRemote(false)).toBe(false);
	});

	it('treats a missing or odd value as local', () => {
		for (const v of [undefined, null, '', {}, [], NaN]) expect(isRemote(v)).toBe(false);
	});
});
