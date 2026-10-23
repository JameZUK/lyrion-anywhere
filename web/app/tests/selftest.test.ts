import { PROBES, SELFTEST_VERSION, chooseVariant, loadCached, mimeFor, saveCached, type SelfTest } from '#lib/player/selftest';

const result = (ok: (rate: number, code: number) => boolean): SelfTest => ({
	version: SELFTEST_VERSION,
	userAgent: 'UA',
	source: 'managed',
	mime: 'audio/mp4; codecs="fLaC"',
	results: PROBES.map((p) => ({ ...p, ok: ok(p.rate, p.code) })),
});

test('the default variant is used when it plays everything', () => {
	expect(chooseVariant(result(() => true))).toEqual({ media: true, code: 0x00, highRates: true });
});

test('a high-rate variant is found when the default fails above 65,535 Hz', () => {
	// Like Safari might: 44.1 kHz fine, 96 kHz only with the srat box.
	const t = result((rate, code) => rate === 44_100 || code === 0x10);
	expect(chooseVariant(t)).toEqual({ media: true, code: 0x10, highRates: true });
});

test('low rates only, or nothing at all, are reported as such', () => {
	expect(chooseVariant(result((rate) => rate === 44_100))).toEqual({ media: true, code: 0x00, highRates: false });
	expect(chooseVariant(result(() => false))).toEqual({ media: false, code: 0, highRates: false });
	expect(chooseVariant(null)).toEqual({ media: false, code: 0, highRates: false });
	// Brand matters: only iso5 plays.
	expect(chooseVariant(result((rate, code) => rate === 44_100 && code === 0x01))).toMatchObject({ media: true, code: 0x01 });
});

test('cached results are reused only for the same browser and test version', () => {
	const m = new Map<string, string>();
	const store = { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
	const t = result(() => true);
	saveCached(t, store);
	expect(loadCached('UA', store)).toEqual(t);
	expect(loadCached('Another browser/2.0', store)).toBeNull();
	m.set('anywhere.mse.selftest', JSON.stringify({ ...t, version: SELFTEST_VERSION - 1 }));
	expect(loadCached('UA', store)).toBeNull();
	m.set('anywhere.mse.selftest', 'not json');
	expect(loadCached('UA', store)).toBeNull();
});

test('other FLAC frame styles are used when the default one is refused', () => {
	// Like Safari might: no variable block sizes, but fixed blocks play (24-bit, all rates).
	expect(chooseVariant(result((_, code) => (code & 0x300) === 0x100))).toEqual({ media: true, code: 0x100, highRates: true });
	// Only fixed-block 16-bit at 44.1 kHz with the iso5 brand.
	expect(chooseVariant(result((rate, code) => rate === 44_100 && code === 0x301))).toEqual({ media: true, code: 0x301, highRates: false });
	// The best frame style that plays wins, even if a later one also plays high rates.
	expect(chooseVariant(result((rate, code) => (code === 0x200 && rate === 44_100) || code >= 0x300))).toEqual({ media: true, code: 0x200, highRates: false });
});

test('every frame style is probed at both rates', () => {
	for (const style of [0x000, 0x100, 0x200, 0x300, 0x400, 0x600]) {
		const mine = PROBES.filter((p) => (p.code & 0x700) === style);
		expect(new Set(mine.map((p) => p.rate))).toEqual(new Set([44_100, 96_000]));
	}
	expect(new Set(PROBES.map((p) => `${p.rate}:${p.code}`)).size).toBe(PROBES.length);
});

test('Apple Lossless is the last resort, and is appended under its own type', () => {
	// Like iPadOS Safari might: every FLAC style refused, ALAC accepted at all rates.
	expect(chooseVariant(result((_, code) => (code & 0x400) !== 0))).toEqual({ media: true, code: 0x400, highRates: true });
	// Only 16-bit ALAC.
	expect(chooseVariant(result((_, code) => (code & 0x600) === 0x600))).toEqual({ media: true, code: 0x600, highRates: true });
	// FLAC still wins when it plays.
	expect(chooseVariant(result(() => true)).code).toBe(0x00);
	expect(mimeFor(0x400, 'audio/mp4; codecs="fLaC"')).toBe('audio/mp4; codecs="alac"');
	expect(mimeFor(0x611, 'audio/mp4; codecs="fLaC"')).toBe('audio/mp4; codecs="alac"');
	expect(mimeFor(0x311, 'audio/mp4; codecs="fLaC"')).toBe('audio/mp4; codecs="fLaC"');
});
