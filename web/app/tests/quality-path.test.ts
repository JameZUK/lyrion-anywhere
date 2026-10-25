// The TypeScript port of the shared core's `quality::describe` (docs/plans/signal-path-redesign.md
// decision 7): the web app must produce exactly the core's diagram for every shared vector,
// and must never throw or produce long strings on any input.
import { readFileSync } from 'node:fs';
import { clean, codec, codecName, describe, fmtDb, formatLabel, isHiRes, isInvisible, khz, originText, type QualityInputs } from '#lib/player/quality-path';

interface Vector {
	name: string;
	input: QualityInputs;
	expect: unknown;
}
const vectors: Vector[] = JSON.parse(readFileSync(`${__dirname}/../../../core/tests/fixtures/quality-vectors.json`, 'utf8'));

test('the shared vectors are all there', () => {
	expect(vectors.length).toBeGreaterThanOrEqual(29);
});

test.each(vectors.map((v) => [v.name, v] as const))('vector: %s', (_name, v) => {
	expect(describe(v.input)).toStrictEqual(v.expect);
});

test('khz matches the core', () => {
	const cases: [number, string][] = [
		[44_100, '44.1'],
		[48_000, '48'],
		[88_200, '88.2'],
		[96_000, '96'],
		[176_400, '176.4'],
		[192_000, '192'],
		[22_050, '22.1'],
		[11_025, '11'],
		[8_000, '8'],
		[0, '0'],
		[4_294_967_295, '4294967.3'],
	];
	for (const [hz, s] of cases) expect(khz(hz), String(hz)).toBe(s);
});

test('labels, Hi-Res and the codec table match the core', () => {
	expect(formatLabel('FLAC', true, 24, 96_000, null)).toBe('FLAC 24/96');
	expect(formatLabel('FLAC', true, null, 44_100, null)).toBe('FLAC 44.1 kHz');
	expect(formatLabel('MP3', false, null, null, 320)).toBe('MP3 320');
	expect(formatLabel('Opus', false, null, 48_000, null)).toBe('Opus');
	expect(formatLabel('DSD', true, 1, 2_822_400, null)).toBe('DSD64');
	expect(formatLabel('DSD', true, 1, 5_644_800, null)).toBe('DSD128');
	expect(formatLabel('DSD', true, null, 44_100, null)).toBe('DSD');
	expect(isHiRes(true, 24, 44_100)).toBe(true);
	expect(isHiRes(true, 16, 96_000)).toBe(true);
	expect(isHiRes(true, 16, 48_000)).toBe(false);
	expect(isHiRes(false, 24, 96_000)).toBe(false);
	expect(codec('flc')).toEqual(['FLAC', true]);
	expect(codec(' FLAC ')).toEqual(['FLAC', true]);
	expect(codec('ops')).toEqual(['Opus', false]);
	expect(codec('xyz')).toBeNull();
	expect(codec('constructor')).toBeNull();
	expect(codec('__proto__')).toBeNull();
	expect(codecName('xyz')).toEqual(['XYZ', false]);
	expect(codecName('toolongname')).toBeNull();
	expect(codecName('a/b')).toBeNull();
	expect(codecName('')).toBeNull();
});

test('the reasons for each origin', () => {
	expect(originText({ kind: 'setting', network: 'mobile', level: 'high' })).toBe('High, on mobile data');
	expect(originText({ kind: 'setting', network: 'wifi', level: 'original' })).toBe('Original, on Wi-Fi');
	expect(originText({ kind: 'setting', network: null, level: 'low' })).toBe('Data saver, chosen in Settings');
	expect(originText({ kind: 'override', level: 'original' })).toBe('Original, overridden here');
	expect(originText({ kind: 'automatic', level: 'high' })).toBe('High, chosen automatically');
});

test('gains are written as the core writes them (ties to even)', () => {
	expect(fmtDb(-3.24)).toBe('−3.2 dB');
	expect(fmtDb(1.5)).toBe('+1.5 dB');
	expect(fmtDb(0.25)).toBe('+0.2 dB');
	expect(fmtDb(0.75)).toBe('+0.8 dB');
	expect(fmtDb(-2.25)).toBe('−2.2 dB');
	expect(fmtDb(0.35)).toBe('+0.3 dB');
	expect(fmtDb(6)).toBe('+6.0 dB');
});

test('names are cleaned of control, zero-width and bidi characters and cut to 40', () => {
	const remote = (name: string) => describe({ player: { kind: 'remote', name } }).blocks[1]!.title;
	const t = remote(`  Living\u0007room\n${'x'.repeat(500)}`);
	expect(t.startsWith('Livingroom')).toBe(true);
	expect(Array.from(t).length).toBe(40);
	expect(remote(' \n')).toBe('Player');
	expect(remote('Kit\u202enehc\u200b\u2066\ufeff')).toBe('Kitnehc');
	expect(clean('a\u2028b\u0085c', 40)).toBe('abc');
});

test('the invisible characters are the core\u2019s list, range by range', () => {
	// Each range's ends and a point inside, as in the core's `is_invisible`.
	const ranges: [number, number][] = [
		[0x0000, 0x001f], [0x007f, 0x009f], [0x00ad, 0x00ad], [0x034f, 0x034f], [0x0600, 0x0605], [0x061c, 0x061c],
		[0x06dd, 0x06dd], [0x070f, 0x070f], [0x0890, 0x0891], [0x08e2, 0x08e2], [0x115f, 0x1160], [0x180e, 0x180e],
		[0x200b, 0x200f], [0x2028, 0x202e], [0x2060, 0x206f], [0x3164, 0x3164], [0xfe00, 0xfe0f], [0xfeff, 0xfeff],
		[0xffa0, 0xffa0], [0xfff9, 0xfffb], [0x110bd, 0x110bd], [0x110cd, 0x110cd], [0x13430, 0x1343f],
		[0x1bca0, 0x1bca3], [0x1d173, 0x1d17a], [0xe0000, 0xe007f], [0xe0100, 0xe01ef],
	];
	for (const [lo, hi] of ranges) {
		for (const cp of [lo, Math.floor((lo + hi) / 2), hi]) {
			const c = String.fromCodePoint(cp);
			expect(isInvisible(c), cp.toString(16)).toBe(true);
			expect(clean(`a${c}b`, 40), cp.toString(16)).toBe('ab');
		}
		// The neighbours outside each range are kept, unless they are themselves in the list.
		for (const cp of [lo - 1, hi + 1]) {
			if (cp < 0 || ranges.some(([a, b]) => cp >= a && cp <= b)) continue;
			expect(isInvisible(String.fromCodePoint(cp)), cp.toString(16)).toBe(false);
		}
	}
	// Ordinary letters, spaces, emoji and joiners that draw are kept.
	for (const c of ['a', '\u00e9', ' ', '\u00a0', '\u{1f3b5}', '\u4e2d', '\u0627']) expect(isInvisible(c), c).toBe(false);
	expect(isInvisible('ab')).toBe(false);
	expect(isInvisible('')).toBe(false);
	// The cut counts code points, as the core's does, and the end is trimmed after it.
	expect(clean('\u{1f3b5}'.repeat(50), 40)).toBe('\u{1f3b5}'.repeat(40));
	expect(clean(`${'x'.repeat(39)} y`, 40)).toBe('x'.repeat(39));
	expect(clean('\u{e0041}\u{e0042}Kitchen\u{fe0f}\u3164', 40)).toBe('Kitchen');
});

test('the device note: the name as given, then the clauses; only clauses are capitalised', () => {
	const note = (device: object) => describe({ device }).blocks.find((b) => b.kind === 'device')!.note;
	expect(note({ name: 'moto buds' })).toBe('moto buds');
	expect(note({ name: 'iBasso DC03', bit_perfect: true })).toBe('iBasso DC03, bit-perfect');
	expect(note({ name: ' \u200b ', resampled: true, rate: 48_000 })).toBe('Resampled to 48 kHz');
	expect(note({ resampled: true, dsp_active: true })).toBe('Resampled, processed by DSP you turned on');
	expect(note({})).toBe('');
});

test('nothing known gives no badge', () => {
	const d = describe({});
	expect(d.badge).toBeNull();
	expect(d.blocks.map((b) => b.kind)).toEqual(['file', 'gateway', 'device']);
});

// ---- robustness: any input, however odd, gives a short, clean diagram --------------------

function rng(seed: number) {
	let s = seed >>> 0;
	return () => {
		s = (s * 1664525 + 1013904223) >>> 0;
		return s / 2 ** 32;
	};
}

test('describe never throws and keeps every string short on random inputs', () => {
	const r = rng(0x5eed);
	const pick = <T>(xs: readonly T[]): T => xs[Math.floor(r() * xs.length)]!;
	const nums = [null, undefined, 0, -1, 1, 8, 16, 24, 32, 64, 65, 999, 1000, 44_100, 48_000, 96_000, 2_822_400, 50_000_001, 1.5, NaN, Infinity, -Infinity, 2 ** 53, '96000', true];
	const strs = [null, undefined, '', ' ', '\u{e0041}\u00adx\ufe0f', 'flc', 'FLAC', 'ops', 'opus', 'ogg', 'mp3', 'pcm', 'dsf', 'alc', 'xyz', 'Opus', 'Vorbis', 'PCM (float)', '\u202e\u200b', 'a'.repeat(300), '\u0000\u0007x', 42, {}, []];
	const any = () => (r() < 0.5 ? pick(nums) : pick(strs));
	const longest = (v: unknown): number => {
		if (typeof v === 'string') {
			expect(Array.from(v).some(isInvisible), v).toBe(false);
			return v.length;
		}
		if (Array.isArray(v)) return Math.max(0, ...v.map(longest));
		if (v && typeof v === 'object') return Math.max(0, ...Object.values(v).map(longest));
		return 0;
	};
	for (let n = 0; n < 4000; n++) {
		const input = {
			player: pick([undefined, { kind: 'local' }, { kind: 'remote', name: pick(strs) }, { kind: 'offline', format: pick(strs) }]),
			file: r() < 0.2 ? null : { kind: pick(strs), rate: any(), bits: any(), kbps: any(), channels: any() },
			served: r() < 0.3 ? null : { codec: pick(strs), rate: any(), bits: any(), channels: any(), gain_db: pick([null, -3.2, 0, 0.04, 0.25, 101, NaN, -Infinity, 1e300, '3']) },
			sent: pick([null, { mode: 'passthrough' }, { mode: 'opus', kbps: any(), level: pick(['high', 'low', null, 'x']) }, { mode: 'x' }]),
			decoded: r() < 0.2 ? null : { codec: pick(strs), rate: any(), bits: any(), channels: any(), lossless: pick([true, false, 'yes', undefined]) },
			device: r() < 0.3 ? null : { name: pick(strs), rate: any(), bit_perfect: r() < 0.3, engine_bit_perfect: r() < 0.5, resampled: r() < 0.5, dsp_active: r() < 0.5 },
			origin: pick([null, { kind: 'setting', network: pick(['wifi', 'mobile', null]), level: pick(['original', 'high', 'low']) }, { kind: 'override', level: 'low' }, { kind: 'automatic', level: 'high' }]),
			seekable: r() < 0.5,
		} as unknown as QualityInputs;
		let d: ReturnType<typeof describe> | undefined;
		expect(() => (d = describe(input)), JSON.stringify(input)).not.toThrow();
		expect(longest(d)).toBeLessThanOrEqual(120);
	}
});
