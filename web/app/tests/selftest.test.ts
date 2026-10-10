import { PROBES, SELFTEST_VERSION, chooseVariant, loadCached, saveCached, type SelfTest } from '$lib/player/selftest';

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
