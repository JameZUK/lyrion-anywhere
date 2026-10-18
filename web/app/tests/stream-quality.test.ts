// Streaming quality in this browser (docs/plans/streaming-quality.md): the automatic choice
// from the Network Information API, the preference kept in this browser, and hostile input.
import {
	LEVEL_KBPS,
	LEVEL_LABELS,
	PREFERENCES,
	autoNote,
	autoQuality,
	connection,
	effectiveQuality,
	loadPreference,
	reencoded,
	sanitisePreference,
	savePreference,
	streamOf,
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

test('Automatic explains itself per browser', () => {
	// Chrome on Android reports its connection; Firefox and Safari (so iPhone and iPad) have
	// no Network Information API, and there Automatic can only ever mean Original.
	expect(autoNote({ type: 'cellular' })).toMatch(/^High on a mobile connection/);
	expect(autoNote({})).toMatch(/^High on a mobile connection/);
	for (const none of [null, undefined]) {
		expect(autoNote(none)).toMatch(/^Always Original in this browser/);
		expect(autoNote(none)).toMatch(/Choose High or Data saver yourself/);
	}
	expect(LEVEL_KBPS).toEqual({ original: null, high: 160, low: 96 });
	expect(LEVEL_LABELS).toEqual({ original: 'Original', high: 'High', low: 'Data saver' });
});

test("the engine's source is checked before it reaches the screen", () => {
	expect(streamOf({ codec: 'Opus', lossless: false, bits: null, channels: 2 })).toEqual({ codec: 'Opus', lossless: false, bits: null });
	expect(streamOf({ codec: 'FLAC', lossless: true, bits: 24 })).toEqual({ codec: 'FLAC', lossless: true, bits: 24 });
	expect(streamOf({ codec: 'PCM (float)', lossless: true })).toMatchObject({ codec: 'PCM (float)', bits: null });
	for (const bad of [null, undefined, 'Opus', 42, {}, [], { codec: '' }, { codec: 42 }, { codec: '<b>Opus</b>' }, { codec: 'x'.repeat(21) }, { codec: ' Opus' }, { codec: 'Opus\n' }, { codec: 'Opus;rm' }]) {
		expect(streamOf(bad), JSON.stringify(bad)).toBeNull();
	}
	// Only a true boolean counts as lossless; bits must be a sane integer.
	expect(streamOf({ codec: 'FLAC', lossless: 'yes' })!.lossless).toBe(false);
	for (const bits of [0, 65, 1.5, '24', NaN, -8, Infinity]) expect(streamOf({ codec: 'FLAC', lossless: true, bits })!.bits, String(bits)).toBeNull();
});

test('re-encoding is recognised only for what the gateway does: FLAC to Opus for a lower level', () => {
	const opus = { codec: 'Opus', lossless: false, bits: null };
	expect(reencoded('FLAC', opus, 'high')).toBe(true);
	expect(reencoded('FLAC', opus, 'original')).toBe(true); // the engine's word beats the level asked for
	expect(reencoded('Opus', opus, 'high')).toBe(false);
	expect(reencoded('MP3', { codec: 'MP3', lossless: false, bits: null }, 'high')).toBe(false);
	expect(reencoded('FLAC', { codec: 'FLAC', lossless: true, bits: 16 }, 'high')).toBe(false);
	expect(reencoded(null, opus, 'high')).toBe(true);
	expect(reencoded(null, opus, 'low')).toBe(true);
	expect(reencoded(null, opus, 'original')).toBe(false);
	expect(reencoded(null, opus, null)).toBe(false);
	expect(reencoded('FLAC', null, 'high')).toBe(false);
});
