// Streaming quality in this browser (docs/plans/streaming-quality.md): the automatic choice
// from the Network Information API, the preference kept in this browser, and hostile input.
import {
	PREFERENCES,
	autoQuality,
	connection,
	effectiveQuality,
	loadPreference,
	sanitisePreference,
	savePreference,
} from '$lib/player/stream-quality';

function memStore() {
	const m = new Map<string, string>();
	return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
}

describe('automatic', () => {
	test.each([
		[{ type: 'cellular' }, 'high'],
		[{ type: 'cellular', saveData: true }, 'low'],
		[{ type: 'wifi', saveData: true }, 'low'],
		[{ type: 'wifi' }, 'original'],
		[{ type: 'ethernet' }, 'original'],
		[{}, 'original'],
		[null, 'original'], // Safari: no Network Information API
		[undefined, 'original'],
		[{ type: 'cellular', saveData: 'yes' as unknown as boolean }, 'high'],
	])('%o → %s', (c, want) => expect(autoQuality(c)).toBe(want));

	test('a chosen level wins over the network', () => {
		expect(effectiveQuality('original', { type: 'cellular' })).toBe('original');
		expect(effectiveQuality('low', { type: 'wifi' })).toBe('low');
		expect(effectiveQuality('auto', { type: 'cellular' })).toBe('high');
	});
});

test('the preference is Automatic unless changed, and checked when read', () => {
	const s = memStore();
	expect(loadPreference(s)).toBe('auto');
	for (const p of PREFERENCES) {
		savePreference(p, s);
		expect(loadPreference(s)).toBe(p);
	}
	s.setItem('anywhere.stream.quality', 'ultra');
	expect(loadPreference(s)).toBe('auto');
	for (const bad of ['', 'HIGH', null, 5, {}, '__proto__', 'toString']) expect(sanitisePreference(bad)).toBe('auto');
	const denied = () => {
		throw new Error('denied');
	};
	expect(loadPreference({ getItem: denied, setItem: denied })).toBe('auto');
	expect(() => savePreference('low', { getItem: denied, setItem: denied })).not.toThrow();
	// A hostile value is never stored as-is.
	savePreference('ultra' as never, s);
	expect(s.getItem('anywhere.stream.quality')).toBe('auto');
});

test('the connection object, where the browser has one', () => {
	expect(connection({})).toBeNull();
	expect(connection(undefined)).toBeNull();
	expect(connection({ connection: 'x' })).toBeNull();
	const c = { type: 'cellular' };
	expect(connection({ connection: c })).toBe(c);
});
