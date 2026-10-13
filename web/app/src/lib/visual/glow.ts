// The Glow style of the visualiser (docs/plans/wave2-design.md §4.1, reworked 2026-10-13):
// a soft halo behind the cover in the artwork's own colours that kicks on each beat, rather
// than following the level. The Android app draws the same design with the same numbers, so
// every constant here is shared; change them in both places together.
//
// Pure functions and small state machines only (no DOM): the beat (onset) detector on the
// low bands of each visualiser frame, the pulse envelope each beat starts, what the pulse and
// the smoothed bass make of the halo, and the artwork's dominant colours.

/** The low bands (0–4 of 32, about 40–110 Hz) that carry the kick. */
export const BASS_BANDS = 5;
/** The running mean of bass energy covers this long. */
export const MEAN_WINDOW_MS = 1500;
/** An onset: bass energy above this many times the running mean, and rising. */
export const ONSET_RATIO = 1.3;
/** No second onset within this long of one. */
export const REFRACTORY_MS = 150;
/** Quieter than this is silence: never a beat. */
export const ENERGY_FLOOR = 0.02;
/** The mean needs this many earlier frames before it is trusted. */
export const MIN_HISTORY = 3;

/** The pulse rises over this long… */
export const ATTACK_MS = 30;
/** …then decays exponentially with this time constant. */
export const DECAY_MS = 350;

/** The halo's radius scale and opacity, at rest and at the top of a pulse. */
export const HALO_RADIUS: readonly [number, number] = [1.0, 1.25];
export const HALO_OPACITY: readonly [number, number] = [0.35, 0.85];
/** The cover's scale, at rest and at the top of a pulse (kept very small). */
export const COVER_SCALE: readonly [number, number] = [1.0, 1.02];
/** How much the smoothed bass moves the halo between beats (a gentle breath). */
export const BREATH_WEIGHT = 0.15;
/** One turn of the halo's lobes takes this long while playing. */
export const ROTATION_MS = 24_000;

const finite = (x: unknown, fallback = 0): number => (typeof x === 'number' && Number.isFinite(x) ? x : fallback);
const unit = (x: unknown): number => Math.min(1, Math.max(0, finite(x)));

/** The bass energy of one frame: the mean of its low bands, 0–1 (garbage counts as 0). */
export function bassEnergy(bands: ArrayLike<number> | null | undefined): number {
	if (!bands || typeof bands.length !== 'number' || bands.length <= 0) return 0;
	const n = Math.min(BASS_BANDS, bands.length);
	let sum = 0;
	for (let i = 0; i < n; i++) sum += unit(bands[i]);
	return sum / n;
}

/**
 * Finds beats in the bass: an onset is a frame whose bass energy is more than ONSET_RATIO
 * times its running mean over the last MEAN_WINDOW_MS and higher than the frame before, at
 * least REFRACTORY_MS after the last onset. A steady tone never qualifies (it equals its own
 * mean), and neither does silence.
 */
export class BeatDetector {
	private times: number[] = [];
	private energies: number[] = [];
	private sum = 0;
	private previous = 0;
	private lastOnset = -Infinity;
	private lastT = -Infinity;

	/** Feed one frame (its bands, and when it arrived in ms); true when it is an onset. */
	step(bands: ArrayLike<number> | null | undefined, tMs: number): boolean {
		const t = finite(tMs, NaN);
		if (!Number.isFinite(t)) return false;
		// Time going backwards (a new clock, a bad value): start again.
		if (t < this.lastT) this.reset();
		this.lastT = t;
		const e = bassEnergy(bands);
		// Drop what has left the window.
		while (this.times.length && t - this.times[0]! > MEAN_WINDOW_MS) {
			this.times.shift();
			this.sum -= this.energies.shift()!;
		}
		const mean = this.times.length ? Math.max(0, this.sum / this.times.length) : 0;
		const onset =
			this.times.length >= MIN_HISTORY &&
			e > ENERGY_FLOOR &&
			e > mean * ONSET_RATIO &&
			e > this.previous &&
			t - this.lastOnset >= REFRACTORY_MS;
		if (onset) this.lastOnset = t;
		this.times.push(t);
		this.energies.push(e);
		this.sum += e;
		this.previous = e;
		return onset;
	}

	reset() {
		this.times = [];
		this.energies = [];
		this.sum = 0;
		this.previous = 0;
		this.lastOnset = -Infinity;
		this.lastT = -Infinity;
	}
}

/**
 * The pulse a beat starts: up to 1 over ATTACK_MS (from wherever it was), then down
 * exponentially with time constant DECAY_MS. 0–1, 0 before the first beat.
 */
export class PulseEnvelope {
	private kickAt = -Infinity;
	private from = 0;

	/** A beat at `tMs`. */
	kick(tMs: number) {
		const t = finite(tMs, NaN);
		if (!Number.isFinite(t)) return;
		this.from = this.value(t);
		this.kickAt = t;
	}

	/** The envelope at `tMs`. */
	value(tMs: number): number {
		const t = finite(tMs, NaN);
		if (!Number.isFinite(t) || !Number.isFinite(this.kickAt)) return 0;
		const since = t - this.kickAt;
		if (since < 0) return unit(this.from);
		if (since < ATTACK_MS) return unit(this.from + (1 - this.from) * (since / ATTACK_MS));
		return unit(Math.exp(-(since - ATTACK_MS) / DECAY_MS));
	}

	reset() {
		this.kickAt = -Infinity;
		this.from = 0;
	}
}

export interface GlowStyle {
	/** The halo's radius, as a scale of its resting size. */
	radius: number;
	/** The halo's opacity. */
	opacity: number;
	/** The cover's scale. */
	cover: number;
}

const lerp = (range: readonly [number, number], f: number) => range[0] + (range[1] - range[0]) * f;

/**
 * What the halo and the cover look like for a pulse (0–1) and the smoothed bass (0–1): the
 * pulse drives them, and the bass adds a gentle breath (BREATH_WEIGHT) to the halo between
 * beats. The cover follows the pulse only.
 */
export function glowStyle(pulse: number, bass = 0): GlowStyle {
	const p = unit(pulse);
	const halo = unit(p + BREATH_WEIGHT * unit(bass));
	return { radius: lerp(HALO_RADIUS, halo), opacity: lerp(HALO_OPACITY, halo), cover: lerp(COVER_SCALE, p) };
}

/** The halo at rest (no music, or reduced motion). */
export const GLOW_REST: GlowStyle = glowStyle(0, 0);

// --- the artwork's colours ---------------------------------------------------------------

export interface Rgb {
	r: number;
	g: number;
	b: number;
}

/** Pixels less saturated than this (HSL) are greys: they don't make a halo. */
export const MIN_SATURATION = 0.25;
/** Nor do near-black or near-white ones. */
export const LIGHTNESS_RANGE: readonly [number, number] = [0.12, 0.9];
/** Fewer usable pixels than this share of the image: use the theme's accent instead. */
export const MIN_SHARE = 0.05;
/** At most this many colours. */
export const MAX_COLOURS = 3;
const ITERATIONS = 10;

/** HSL saturation and lightness of a colour (0–255 channels). */
export function saturationLightness(c: Rgb): { s: number; l: number } {
	const r = c.r / 255;
	const g = c.g / 255;
	const b = c.b / 255;
	const max = Math.max(r, g, b);
	const min = Math.min(r, g, b);
	const l = (max + min) / 2;
	const d = max - min;
	const s = d === 0 ? 0 : d / (1 - Math.abs(2 * l - 1));
	return { s: Math.min(1, Math.max(0, s)), l };
}

/** Saturated enough, and neither too dark nor too light, to colour the halo. */
export function usable(c: Rgb): boolean {
	const { s, l } = saturationLightness(c);
	return s >= MIN_SATURATION && l >= LIGHTNESS_RANGE[0] && l <= LIGHTNESS_RANGE[1];
}

const dist2 = (a: Rgb, b: Rgb) => (a.r - b.r) ** 2 + (a.g - b.g) ** 2 + (a.b - b.b) ** 2;

/**
 * The 2–3 dominant, saturated colours of an image's pixels (RGBA bytes, as from
 * getImageData on a small copy of the cover, e.g. 32 × 32), biggest group first: k-means on
 * the usable pixels, seeded deterministically (the most saturated, then the farthest each
 * time). Empty when too little of the image is colourful (a grey or black-and-white cover):
 * the caller then uses the theme's accent.
 */
export function dominantColours(rgba: ArrayLike<number> | null | undefined, k: number = MAX_COLOURS): Rgb[] {
	if (!rgba || typeof rgba.length !== 'number') return [];
	const total = Math.floor(rgba.length / 4);
	if (total <= 0) return [];
	const want = Math.max(1, Math.min(MAX_COLOURS, Math.floor(finite(k, MAX_COLOURS))));
	const px: Rgb[] = [];
	for (let i = 0; i < total; i++) {
		const a = finite(rgba[i * 4 + 3], 255);
		if (a < 128) continue;
		const c = { r: unit(finite(rgba[i * 4]) / 255) * 255, g: unit(finite(rgba[i * 4 + 1]) / 255) * 255, b: unit(finite(rgba[i * 4 + 2]) / 255) * 255 };
		if (usable(c)) px.push(c);
	}
	if (px.length < Math.max(1, total * MIN_SHARE)) return [];

	// Seeds: the most saturated pixel, then each time the pixel farthest from every seed.
	let first = px[0]!;
	let best = -1;
	for (const p of px) {
		const s = saturationLightness(p).s;
		if (s > best) {
			best = s;
			first = p;
		}
	}
	const centres: Rgb[] = [{ ...first }];
	while (centres.length < want) {
		let far: Rgb | null = null;
		let farD = 0;
		for (const p of px) {
			const d = Math.min(...centres.map((c) => dist2(p, c)));
			if (d > farD) {
				farD = d;
				far = p;
			}
		}
		if (!far) break; // every pixel is already a centre
		centres.push({ ...far });
	}

	const assign = new Array<number>(px.length).fill(0);
	for (let it = 0; it < ITERATIONS; it++) {
		for (let i = 0; i < px.length; i++) {
			let bi = 0;
			let bd = Infinity;
			for (let j = 0; j < centres.length; j++) {
				const d = dist2(px[i]!, centres[j]!);
				if (d < bd) {
					bd = d;
					bi = j;
				}
			}
			assign[i] = bi;
		}
		const sums = centres.map(() => ({ r: 0, g: 0, b: 0, n: 0 }));
		for (let i = 0; i < px.length; i++) {
			const s = sums[assign[i]!]!;
			s.r += px[i]!.r;
			s.g += px[i]!.g;
			s.b += px[i]!.b;
			s.n++;
		}
		sums.forEach((s, j) => {
			if (s.n) centres[j] = { r: s.r / s.n, g: s.g / s.n, b: s.b / s.n };
		});
	}
	const counts = centres.map(() => 0);
	for (const a of assign) counts[a]!++;
	return centres
		.map((c, j) => ({ c: { r: Math.round(c.r), g: Math.round(c.g), b: Math.round(c.b) }, n: counts[j]! }))
		.filter((x) => x.n >= Math.max(1, px.length * MIN_SHARE) && usable(x.c))
		.sort((a, b) => b.n - a.n)
		.map((x) => x.c);
}

/** A colour for CSS. */
export function cssColour(c: Rgb): string {
	return `rgb(${c.r} ${c.g} ${c.b})`;
}

/**
 * The three lobes' colours: the artwork's (repeated round when there are fewer than three),
 * or shades of the theme's accent when the cover has none to give.
 */
export function haloColours(colours: readonly Rgb[]): string[] {
	if (!colours.length) return ['var(--accent)', 'color-mix(in srgb, var(--accent) 70%, white)', 'color-mix(in srgb, var(--accent) 70%, black)'];
	return [0, 1, 2].map((i) => cssColour(colours[i % colours.length]!));
}

/** The colours found for each cover, so an album is analysed once (oldest forgotten first). */
export class ColourCache {
	private map = new Map<string, Rgb[]>();
	constructor(private readonly max = 50) {}

	get(key: string): Rgb[] | undefined {
		const v = this.map.get(key);
		if (v) {
			this.map.delete(key);
			this.map.set(key, v);
		}
		return v;
	}

	set(key: string, colours: Rgb[]) {
		this.map.delete(key);
		this.map.set(key, colours);
		while (this.map.size > this.max) this.map.delete(this.map.keys().next().value!);
	}

	get size() {
		return this.map.size;
	}
}
