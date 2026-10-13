// The Glow halo's logic (lib/visual/glow.ts): the beat detector on the low bands, the pulse
// envelope a beat starts, what they make of the halo, and the cover's dominant colours.
import {
	ATTACK_MS,
	BeatDetector,
	ColourCache,
	COVER_SCALE,
	DECAY_MS,
	GLOW_REST,
	HALO_OPACITY,
	HALO_RADIUS,
	PulseEnvelope,
	REFRACTORY_MS,
	bassEnergy,
	dominantColours,
	glowStyle,
	haloColours,
	saturationLightness,
	type Rgb,
} from '$lib/visual/glow';

const FRAME = 1000 / 30;

/** 32 bands with the low five at `bass` and the rest at `rest`. */
function bands(bass: number, rest = 0.2): Float32Array {
	const b = new Float32Array(32).fill(rest);
	for (let i = 0; i < 5; i++) b[i] = bass;
	return b;
}

/** Run frames through a detector; the times (ms) of its onsets. */
function run(d: BeatDetector, levels: (t: number, i: number) => number, ms: number): number[] {
	const onsets: number[] = [];
	for (let i = 0, t = 0; t <= ms; i++, t = i * FRAME) if (d.step(bands(levels(t, i)), t)) onsets.push(t);
	return onsets;
}

describe('beat detector', () => {
	test('bass energy is the mean of bands 0–4, cleaned', () => {
		expect(bassEnergy(bands(0.5, 1))).toBeCloseTo(0.5);
		expect(bassEnergy([1, 1])).toBe(1);
		expect(bassEnergy([NaN, Infinity, -3, 7, 0.5])).toBeCloseTo((0 + 0 + 0 + 1 + 0.5) / 5);
		expect(bassEnergy(null)).toBe(0);
		expect(bassEnergy(undefined)).toBe(0);
		expect(bassEnergy([])).toBe(0);
	});

	test('a steady tone gives no repeated onsets', () => {
		const d = new BeatDetector();
		expect(run(d, () => 0.6, 10_000)).toHaveLength(0);
		// A slowly swelling one neither: it never jumps 30% above its own recent mean.
		const s = new BeatDetector();
		expect(run(s, (t) => 0.3 + 0.3 * (t / 10_000), 10_000)).toHaveLength(0);
	});

	test('silence gives none', () => {
		expect(run(new BeatDetector(), () => 0, 5000)).toHaveLength(0);
		// Nor does near-silent noise, however jumpy.
		expect(run(new BeatDetector(), (_t, i) => (i % 3 === 0 ? 0.015 : 0.001), 5000)).toHaveLength(0);
	});

	test('a periodic kick gives one onset per kick', () => {
		// 120 bpm: a kick every 500 ms lasting three frames (rise, peak, fall) over a bed.
		// (At 30 frames a second, 500 ms is exactly 15 frames.)
		const kick = (_t: number, i: number) => [0.7, 0.9, 0.6][i % 15] ?? 0.25;
		const onsets = run(new BeatDetector(), kick, 7990);
		// The first kick comes before the mean is trusted; every later one is found, once.
		const kicks = 15; // at 500, 1000, … 7500
		expect(onsets).toHaveLength(kicks);
		for (let i = 1; i < onsets.length; i++) expect(onsets[i]! - onsets[i - 1]!).toBeGreaterThan(400);
		for (const t of onsets) expect(t % 500).toBeLessThan(2 * FRAME);
	});

	test('the refractory period: no second onset within 150 ms', () => {
		const d = new BeatDetector();
		for (let t = 0; t < 1500; t += FRAME) d.step(bands(0.2), t);
		expect(d.step(bands(0.6), 1500)).toBe(true);
		// Falls and jumps again at once: too soon.
		expect(d.step(bands(0.2), 1520)).toBe(false);
		expect(d.step(bands(0.9), 1540)).toBe(false);
		expect(d.step(bands(0.2), 1600)).toBe(false);
		expect(d.step(bands(0.95), 1500 + REFRACTORY_MS - 1)).toBe(false);
		expect(d.step(bands(0.2), 1660)).toBe(false);
		// After it, a fresh rise counts.
		expect(d.step(bands(0.99), 1500 + REFRACTORY_MS + 30)).toBe(true);
	});

	test('a rise must be rising: a high plateau is one onset', () => {
		const d = new BeatDetector();
		for (let t = 0; t < 1500; t += FRAME) d.step(bands(0.2), t);
		const onsets: boolean[] = [];
		for (let t = 1500; t < 2500; t += FRAME) onsets.push(d.step(bands(0.8), t));
		expect(onsets.filter(Boolean)).toHaveLength(1);
	});

	test('NaN, garbage and backwards time never throw or fire', () => {
		const d = new BeatDetector();
		const junk: unknown[] = [null, undefined, [], [NaN, NaN], 'loud', { length: -1 }, { length: 5 }, new Float32Array(3).fill(Infinity)];
		let fired = 0;
		for (let i = 0; i < 400; i++) {
			const b = junk[i % junk.length] as ArrayLike<number>;
			if (d.step(b, i * FRAME)) fired++;
			if (d.step(bands(0.3), NaN)) fired++;
			if (d.step(bands(0.3), Infinity)) fired++;
		}
		expect(fired).toBe(0);
		// Time going backwards starts again rather than misfiring.
		expect(d.step(bands(1), -5000)).toBe(false);
		// Fuzz: random frames and times never throw, and onsets respect the refractory period.
		const f = new BeatDetector();
		let last = -Infinity;
		for (let i = 0, t = 0; i < 5000; i++) {
			t += Math.random() * 60;
			const b = Float32Array.from({ length: 32 }, () => (Math.random() < 0.05 ? NaN : Math.random() * 3 - 1));
			if (f.step(b, t)) {
				expect(t - last).toBeGreaterThanOrEqual(REFRACTORY_MS);
				last = t;
			}
		}
	});
});

describe('pulse envelope', () => {
	test('nothing before the first beat', () => {
		const e = new PulseEnvelope();
		expect(e.value(0)).toBe(0);
		expect(e.value(1e6)).toBe(0);
	});

	test('attack 30 ms to the top, then an exponential decay of 350 ms', () => {
		const e = new PulseEnvelope();
		e.kick(1000);
		expect(e.value(1000)).toBeCloseTo(0);
		expect(e.value(1000 + ATTACK_MS / 2)).toBeCloseTo(0.5);
		expect(e.value(1000 + ATTACK_MS)).toBeCloseTo(1);
		expect(e.value(1000 + ATTACK_MS + DECAY_MS)).toBeCloseTo(Math.exp(-1));
		expect(e.value(1000 + ATTACK_MS + 3 * DECAY_MS)).toBeCloseTo(Math.exp(-3));
		expect(e.value(1000 + 10_000)).toBeLessThan(0.001);
		// It only ever falls after the top.
		let prev = 1;
		for (let t = 1000 + ATTACK_MS; t < 3000; t += 10) {
			const v = e.value(t);
			expect(v).toBeLessThanOrEqual(prev);
			prev = v;
		}
	});

	test('a beat during the decay rises from where it was, not from 0', () => {
		const e = new PulseEnvelope();
		e.kick(0);
		const mid = e.value(200);
		e.kick(200);
		expect(e.value(200)).toBeCloseTo(mid);
		expect(e.value(200 + ATTACK_MS / 2)).toBeGreaterThan(mid);
		expect(e.value(200 + ATTACK_MS)).toBeCloseTo(1);
	});

	test('bad times are ignored, and it stays 0–1', () => {
		const e = new PulseEnvelope();
		e.kick(NaN);
		expect(e.value(10)).toBe(0);
		e.kick(100);
		for (const t of [NaN, -Infinity, Infinity, -1e9, 1e12]) {
			const v = e.value(t);
			expect(v >= 0 && v <= 1).toBe(true);
		}
		e.reset();
		expect(e.value(200)).toBe(0);
	});
});

describe('halo style', () => {
	test('at rest and at the top of a pulse', () => {
		expect(GLOW_REST).toEqual({ radius: HALO_RADIUS[0], opacity: HALO_OPACITY[0], cover: COVER_SCALE[0] });
		expect(GLOW_REST).toEqual({ radius: 1, opacity: 0.35, cover: 1 });
		const top = glowStyle(1, 0);
		expect(top.radius).toBeCloseTo(1.25);
		expect(top.opacity).toBeCloseTo(0.85);
		expect(top.cover).toBeCloseTo(1.02);
	});

	test('the bass only breathes the halo a little, and never the cover', () => {
		const breath = glowStyle(0, 1);
		expect(breath.radius).toBeGreaterThan(1);
		expect(breath.radius).toBeLessThan(1.05);
		expect(breath.opacity).toBeLessThan(0.45);
		expect(breath.cover).toBe(1);
	});

	test('garbage stays in range', () => {
		for (const [p, b] of [
			[NaN, NaN],
			[5, 5],
			[-3, -3],
			[Infinity, -Infinity],
		]) {
			const g = glowStyle(p!, b!);
			expect(g.radius).toBeGreaterThanOrEqual(1);
			expect(g.radius).toBeLessThanOrEqual(1.25);
			expect(g.opacity).toBeGreaterThanOrEqual(0.35);
			expect(g.opacity).toBeLessThanOrEqual(0.85);
			expect(g.cover).toBeGreaterThanOrEqual(1);
			expect(g.cover).toBeLessThanOrEqual(1.02);
		}
	});
});

/** RGBA bytes from colours, each repeated `n` times. */
function image(parts: [Rgb, number][], alpha = 255): Uint8ClampedArray {
	const px: number[] = [];
	for (const [c, n] of parts) for (let i = 0; i < n; i++) px.push(c.r, c.g, c.b, alpha);
	return Uint8ClampedArray.from(px);
}
const near = (a: Rgb, b: Rgb, tol = 24) => Math.abs(a.r - b.r) <= tol && Math.abs(a.g - b.g) <= tol && Math.abs(a.b - b.b) <= tol;

describe('the cover’s colours', () => {
	const RED = { r: 200, g: 30, b: 40 };
	const BLUE = { r: 30, g: 60, b: 210 };
	const GREEN = { r: 40, g: 180, b: 70 };

	test('k-means finds the dominant colours, biggest first', () => {
		// 32 × 32 = 1024 pixels: mostly red, then blue, some green, with jitter and grey.
		const jitter = (c: Rgb, i: number): Rgb => ({ r: c.r + ((i * 7) % 11) - 5, g: c.g + ((i * 5) % 9) - 4, b: c.b + ((i * 3) % 7) - 3 });
		const parts: [Rgb, number][] = [];
		for (let i = 0; i < 500; i++) parts.push([jitter(RED, i), 1]);
		for (let i = 0; i < 300; i++) parts.push([jitter(BLUE, i), 1]);
		for (let i = 0; i < 124; i++) parts.push([jitter(GREEN, i), 1]);
		parts.push([{ r: 128, g: 128, b: 128 }, 100]);
		const found = dominantColours(image(parts));
		expect(found).toHaveLength(3);
		expect(near(found[0]!, RED)).toBe(true);
		expect(near(found[1]!, BLUE)).toBe(true);
		expect(near(found[2]!, GREEN)).toBe(true);
	});

	test('two colours give two', () => {
		const found = dominantColours(image([[RED, 600], [BLUE, 424]]));
		expect(found).toHaveLength(2);
		expect(near(found[0]!, RED, 2)).toBe(true);
		expect(near(found[1]!, BLUE, 2)).toBe(true);
	});

	test('a grey image gives none: the accent is used instead', () => {
		const greys: [Rgb, number][] = [];
		for (let v = 0; v < 256; v += 4) greys.push([{ r: v, g: v, b: v }, 16]);
		expect(dominantColours(image(greys))).toEqual([]);
		expect(haloColours([])).toEqual(['var(--accent)', expect.stringContaining('var(--accent)'), expect.stringContaining('var(--accent)')]);
		// Black and white, and nearly-black colour, neither.
		expect(dominantColours(image([[{ r: 0, g: 0, b: 0 }, 512], [{ r: 255, g: 255, b: 255 }, 512]]))).toEqual([]);
		expect(dominantColours(image([[{ r: 20, g: 0, b: 5 }, 1024]]))).toEqual([]);
	});

	test('a few coloured pixels in a grey cover are not enough', () => {
		expect(dominantColours(image([[{ r: 120, g: 120, b: 120 }, 1000], [RED, 24]]))).toEqual([]);
	});

	test('transparent pixels are skipped', () => {
		expect(dominantColours(image([[RED, 1024]], 0))).toEqual([]);
	});

	test('the lobes repeat round fewer colours', () => {
		expect(haloColours([RED])).toEqual(['rgb(200 30 40)', 'rgb(200 30 40)', 'rgb(200 30 40)']);
		expect(haloColours([RED, BLUE])).toEqual(['rgb(200 30 40)', 'rgb(30 60 210)', 'rgb(200 30 40)']);
	});

	test('garbage never throws', () => {
		for (const v of [null, undefined, [], [1, 2, 3], new Uint8ClampedArray(0), [NaN, NaN, NaN, NaN], Array(400).fill('x')]) {
			expect(() => dominantColours(v as ArrayLike<number>)).not.toThrow();
		}
		expect(dominantColours(image([[RED, 64]]), NaN)).toHaveLength(1);
		const rnd = Uint8ClampedArray.from({ length: 4096 }, () => Math.floor(Math.random() * 256));
		const found = dominantColours(rnd);
		expect(found.length).toBeLessThanOrEqual(3);
		for (const c of found) expect(saturationLightness(c).s).toBeGreaterThanOrEqual(0.25);
	});

	test('the cache keeps an album’s colours, forgetting the oldest', () => {
		const c = new ColourCache(2);
		c.set('a', [RED]);
		c.set('b', [BLUE]);
		expect(c.get('a')).toEqual([RED]); // a is now the newest
		c.set('c', [GREEN]);
		expect(c.get('b')).toBeUndefined();
		expect(c.get('a')).toEqual([RED]);
		expect(c.get('c')).toEqual([GREEN]);
		expect(c.size).toBe(2);
		// Empty results (grey covers) are kept too, so they are not analysed again.
		c.set('d', []);
		expect(c.get('d')).toEqual([]);
	});
});
