// The visualiser's client side (Wave 2 §4.1) and Now playing's remembered choices (§4.2):
// frames from the worker, VU ballistics, drawing, and the per-device preferences.
import { BANDS, FLOOR_DB, FRAME_LEN, SCOPE_POINTS, parseFrame, silentFrame } from '$lib/visual/frame';
import { BAR_FALL_PER_S, PEAK_HOLD_MS, VU_REFERENCE_MAX, VU_REFERENCE_MIN, VU_RESPONSE_MS, VuBallistics, VuCalibration, fallBars, needlePosition } from '$lib/visual/vu';
import { STYLES, drawBars, drawScope, drawVu, glowLevel, nextStyle, type Ctx, type Palette } from '$lib/visual/draw';
import {
	DEFAULT_NOW_PLAYING,
	NOW_PLAYING_KEY,
	layoutForKey,
	loadNowPlaying,
	saveNowPlaying,
	swipeLayout,
} from '$lib/nowplaying/prefs';

/** A frame as the worker sends it. */
function raw(over: Partial<{ bands: number; scope: number; rms: [number, number]; peak: [number, number]; bass: number }> = {}): Float32Array {
	const f = new Float32Array(FRAME_LEN);
	f.fill(over.bands ?? 0.5, 0, BANDS);
	f.fill(over.scope ?? 0.25, BANDS, BANDS + SCOPE_POINTS);
	const at = BANDS + SCOPE_POINTS;
	[f[at], f[at + 1]] = over.rms ?? [-20, -22];
	[f[at + 2], f[at + 3]] = over.peak ?? [-6, -8];
	f[at + 4] = over.bass ?? 0.7;
	return f;
}

/** A seeded generator, so a failing fuzz case can be replayed. */
function rng(seed: number) {
	let s = seed >>> 0;
	return () => {
		s = (s * 1664525 + 1013904223) >>> 0;
		return s / 2 ** 32;
	};
}

describe('frames from the worker', () => {
	test('a well-formed frame is read as laid out by the core', () => {
		const f = parseFrame(raw())!;
		expect(f.bands).toHaveLength(BANDS);
		expect(f.scope).toHaveLength(SCOPE_POINTS);
		expect(f.bands[0]).toBeCloseTo(0.5);
		expect(f.scope[127]).toBeCloseTo(0.25);
		expect(f.rmsDb).toEqual([-20, -22]);
		expect(f.peakDb).toEqual([-6, -8]);
		expect(f.bass).toBeCloseTo(0.7);
	});

	test('anything else is refused', () => {
		for (const v of [null, undefined, 1, 'frame', {}, [], new Float32Array(FRAME_LEN - 1), new Float32Array(FRAME_LEN + 1), new Float64Array(FRAME_LEN), new Uint8Array(FRAME_LEN)]) {
			expect(parseFrame(v)).toBeNull();
		}
	});

	test('out-of-range and non-finite values are cleaned', () => {
		const f = parseFrame(raw({ bands: 7, scope: -9, rms: [NaN, 12], peak: [Infinity, -1e9], bass: NaN }))!;
		expect(f.bands.every((x) => x === 1)).toBe(true);
		expect(f.scope.every((x) => x === -1)).toBe(true);
		expect(f.rmsDb).toEqual([FLOOR_DB, 0]);
		expect(f.peakDb).toEqual([FLOOR_DB, FLOOR_DB]);
		expect(f.bass).toBe(0);
	});

	test('fuzz: random frames always come out in range', () => {
		const r = rng(7);
		const odd = [NaN, Infinity, -Infinity, 1e38, -1e38, 0, -0];
		for (let n = 0; n < 300; n++) {
			const v = new Float32Array(FRAME_LEN).map(() => (r() < 0.1 ? odd[Math.floor(r() * odd.length)]! : (r() - 0.5) * 400));
			const f = parseFrame(v)!;
			expect([...f.bands].every((x) => x >= 0 && x <= 1)).toBe(true);
			expect([...f.scope].every((x) => x >= -1 && x <= 1)).toBe(true);
			for (const d of [...f.rmsDb, ...f.peakDb]) expect(d >= FLOOR_DB && d <= 0).toBe(true);
			expect(f.bass >= 0 && f.bass <= 1).toBe(true);
		}
	});

	test('silence is a valid frame', () => {
		const s = silentFrame();
		expect(s.bands.every((x) => x === 0)).toBe(true);
		expect(s.rmsDb).toEqual([FLOOR_DB, FLOOR_DB]);
	});
});

describe('VU ballistics', () => {
	/** Step a meter in 10 ms steps for `ms`. */
	function run(m: VuBallistics, rms: number, peak: number, ms: number, from = 0): number {
		let t = from;
		for (; t < from + ms; t += 10) m.step(rms, peak, 10, t + 10);
		return t;
	}

	test('the needle reaches 99% of a rise in about 300 ms, not at once', () => {
		const m = new VuBallistics();
		m.step(-60, -60, 1000, 0); // settled at −60
		expect(m.level).toBeCloseTo(-60, 0);
		run(m, -60 + 40, -10, 30, 0);
		// After 30 ms it is well short of the new level…
		expect(m.level).toBeLessThan(-35);
		run(m, -20, -10, VU_RESPONSE_MS - 30, 30);
		// …and after 300 ms within 1% of the 40 dB step.
		expect(Math.abs(m.level - -20)).toBeLessThanOrEqual(0.4 + 1e-6);
	});

	test('it falls back just as smoothly', () => {
		const m = new VuBallistics();
		m.step(-10, -10, 1000, 0);
		run(m, -50, -50, 50, 0);
		expect(m.level).toBeGreaterThan(-35);
		run(m, -50, -50, VU_RESPONSE_MS - 50, 50);
		expect(Math.abs(m.level - -50)).toBeLessThanOrEqual(0.4 + 1e-6);
	});

	test('the peak holds, then falls, and never sits below the needle', () => {
		const m = new VuBallistics();
		m.step(-30, -3, 10, 0);
		expect(m.peak).toBe(-3);
		const t = run(m, -30, -40, PEAK_HOLD_MS - 50, 0);
		expect(m.peak).toBe(-3);
		run(m, -30, -40, 1000, t);
		expect(m.peak).toBeLessThan(-3);
		expect(m.peak).toBeGreaterThanOrEqual(m.level);
		run(m, -30, -90, 10_000, t + 1000);
		expect(m.peak).toBeCloseTo(m.level, 5);
	});

	test('bad steps and levels never break it', () => {
		const m = new VuBallistics();
		for (const dt of [NaN, -5, Infinity, 1e12]) m.step(NaN, Infinity, dt, NaN);
		expect(Number.isFinite(m.level)).toBe(true);
		expect(Number.isFinite(m.peak)).toBe(true);
		// A huge step (the page was hidden) settles rather than overshooting.
		m.step(-18, -10, 1e9, 0);
		expect(m.level).toBeGreaterThanOrEqual(FLOOR_DB);
		expect(m.level).toBeLessThanOrEqual(0);
		m.reset();
		expect(m.level).toBe(FLOOR_DB);
	});

	test('the scale runs from −20 VU to +3 VU around −18 dBFS', () => {
		expect(needlePosition(-38)).toBe(0);
		expect(needlePosition(-90)).toBe(0);
		expect(needlePosition(-18)).toBeCloseTo(20 / 23);
		expect(needlePosition(-15)).toBe(1);
		expect(needlePosition(0)).toBe(1);
		expect(needlePosition(NaN)).toBe(0);
	});

	test('the calibration follows the mastering level, so the needle swings rather than pins', () => {
		// Regression: a fixed −18 dBFS reference left the needles at the end stop on modern
		// masters (an average near −9 dBFS).
		const settle = (c: VuCalibration, db: number, ms: number) => {
			for (let t = 0; t < ms; t += 33) c.step([db, db], 33);
			return c.reference;
		};
		const loud = new VuCalibration();
		const ref = settle(loud, -9, 15_000);
		expect(ref).toBeCloseTo(-6, 0);
		expect(needlePosition(-9, ref)).toBeCloseTo(17 / 23, 1); // about −3 VU
		expect(needlePosition(-9, ref)).toBeLessThan(needlePosition(-5, ref));
		expect(needlePosition(-4, ref)).toBeGreaterThan(20 / 23); // loud passages reach the red
		// A quiet recording sits in the same place.
		const quiet = new VuCalibration();
		const qref = settle(quiet, -24, 15_000);
		expect(needlePosition(-24, qref)).toBeCloseTo(needlePosition(-9, ref), 1);
		// Silence and gaps leave it alone; garbage is ignored; it stays within its limits.
		settle(loud, -90, 20_000);
		expect(loud.reference).toBeCloseTo(ref, 1);
		loud.step([NaN, Infinity], 33);
		expect(Number.isFinite(loud.reference)).toBe(true);
		expect(settle(new VuCalibration(), -1, 20_000)).toBe(VU_REFERENCE_MAX);
		expect(settle(new VuCalibration(), -50, 20_000)).toBe(VU_REFERENCE_MIN);
		// It settles within a few seconds, then moves slowly (a quiet bar doesn't swing it).
		const c = new VuCalibration();
		settle(c, -9, 3_000);
		expect(c.reference).toBeCloseTo(-6, 0);
		settle(c, -20, 1_000);
		expect(c.reference).toBeGreaterThan(-8);
		c.reset();
		expect(c.reference).toBe(-18);
	});

	test('bars jump up and fall smoothly', () => {
		const shown = new Float32Array([0, 1, 0.5]);
		fallBars(shown, new Float32Array([0.8, 0, NaN]), 100);
		expect(shown[0]).toBeCloseTo(0.8);
		expect(shown[1]).toBeCloseTo(1 - BAR_FALL_PER_S * 0.1);
		expect(shown[2]).toBeCloseTo(0.5 - BAR_FALL_PER_S * 0.1);
		fallBars(shown, new Float32Array(3), 10_000);
		expect([...shown]).toEqual([0, 0, 0]);
	});
});

describe('drawing', () => {
	const palette: Palette = { accent: '#36c', fg: '#fff', muted: '#999', line: '#444', danger: '#e44' };
	/** A context that records what is drawn and checks every coordinate is a number. */
	function fakeCtx() {
		const calls: string[] = [];
		const check = (name: string) => (...args: unknown[]) => {
			for (const a of args) if (typeof a === 'number' && !Number.isFinite(a)) throw new Error(`${name}: ${String(a)}`);
			calls.push(name);
		};
		const ctx = {
			clearRect: check('clearRect'),
			fillRect: check('fillRect'),
			beginPath: check('beginPath'),
			moveTo: check('moveTo'),
			lineTo: check('lineTo'),
			stroke: check('stroke'),
			arc: check('arc'),
			fill: check('fill'),
			fillText: check('fillText'),
			save: check('save'),
			restore: check('restore'),
			fillStyle: '',
			strokeStyle: '',
			lineWidth: 1,
			lineCap: 'butt',
			font: '',
			textAlign: 'left',
			globalAlpha: 1,
		} as unknown as Ctx;
		return { ctx, calls };
	}

	test('bars: one per band, scope: one line through every point, VU: two meters', () => {
		const f = parseFrame(raw())!;
		const a = fakeCtx();
		drawBars(a.ctx, 320, 320, f.bands, palette);
		expect(a.calls.filter((c) => c === 'fillRect')).toHaveLength(BANDS);
		const b = fakeCtx();
		drawScope(b.ctx, 320, 320, f.scope, palette);
		expect(b.calls.filter((c) => c === 'lineTo')).toHaveLength(1 + SCOPE_POINTS - 1);
		const c = fakeCtx();
		drawVu(c.ctx, 400, 200, [-18, -30], [-3, -12], palette);
		expect(c.calls.filter((x) => x === 'fillText')).toHaveLength(2);
	});

	test('odd sizes and values never throw', () => {
		const r = rng(3);
		for (let n = 0; n < 100; n++) {
			const w = [0, -1, 1, 37, 1000][n % 5]!;
			const h = [0, 1, 300, -5, 2][n % 5]!;
			const vals = Array.from({ length: [0, 1, 32, 128][n % 4]! }, () => (r() < 0.2 ? NaN : (r() - 0.5) * 10));
			const { ctx } = fakeCtx();
			drawBars(ctx, w, h, vals, palette);
			drawScope(ctx, w, h, vals, palette);
			drawVu(ctx, w, h, [vals[0] ?? NaN, NaN], [Infinity, -Infinity], palette);
		}
	});

	test('the glow eases towards the bass: quick up, slower down, 0–1 always', () => {
		const up = glowLevel(0, { bass: 1 }, 40);
		expect(up).toBeGreaterThan(0.5);
		const down = glowLevel(1, { bass: 0 }, 40);
		expect(down).toBeGreaterThan(0.7);
		expect(glowLevel(0.5, null, 10_000)).toBeCloseTo(0);
		for (const v of [glowLevel(NaN, { bass: NaN }, NaN), glowLevel(2, { bass: 5 }, 1e9)]) expect(v >= 0 && v <= 1).toBe(true);
	});

	test('styles go round in both directions', () => {
		expect(STYLES).toEqual(['bars', 'scope', 'glow', 'vu']);
		expect(nextStyle('bars')).toBe('scope');
		expect(nextStyle('vu')).toBe('bars');
		expect(nextStyle('bars', -1)).toBe('vu');
	});
});

describe('Now playing preferences', () => {
	const store = () => {
		const m = new Map<string, string>();
		return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v), m };
	};

	test('the visualiser is off and the layout is Art by default', () => {
		expect(loadNowPlaying(store())).toEqual(DEFAULT_NOW_PLAYING);
		expect(DEFAULT_NOW_PLAYING.visualiser).toBe(false);
		expect(DEFAULT_NOW_PLAYING.layout).toBe('art');
	});

	test('choices are kept and read back', () => {
		const s = store();
		saveNowPlaying({ layout: 'info', visualiser: true, style: 'vu', queuePanel: false }, s);
		expect(loadNowPlaying(s)).toEqual({ layout: 'info', visualiser: true, style: 'vu', queuePanel: false });
	});

	test('junk, oversize or blocked storage gives the defaults', () => {
		for (const v of ['{', '[]', '"art"', 'null', JSON.stringify({ layout: 'evil', visualiser: 'yes', style: '<script>', queuePanel: 1 }), JSON.stringify({ layout: 'lyrics', pad: 'x'.repeat(2000) })]) {
			const s = store();
			s.m.set(NOW_PLAYING_KEY, v);
			const p = loadNowPlaying(s);
			expect(['art', 'lyrics', 'info']).toContain(p.layout);
			expect(typeof p.visualiser).toBe('boolean');
			expect(['bars', 'scope', 'glow', 'vu']).toContain(p.style);
		}
		const s = store();
		s.m.set(NOW_PLAYING_KEY, JSON.stringify({ layout: 'evil', visualiser: 'yes' }));
		expect(loadNowPlaying(s)).toEqual(DEFAULT_NOW_PLAYING);
		const throwing = {
			getItem: () => {
				throw new Error('blocked');
			},
			setItem: () => {
				throw new Error('full');
			},
		};
		expect(loadNowPlaying(throwing)).toEqual(DEFAULT_NOW_PLAYING);
		expect(() => saveNowPlaying(DEFAULT_NOW_PLAYING, throwing)).not.toThrow();
		expect(loadNowPlaying(undefined)).toEqual(DEFAULT_NOW_PLAYING);
	});

	test('keys 1, 2, 3 and sideways swipes pick layouts', () => {
		expect(['1', '2', '3', '4', 'a', ''].map(layoutForKey)).toEqual(['art', 'lyrics', 'info', null, null, null]);
		expect(swipeLayout('art', -80)).toBe('lyrics');
		expect(swipeLayout('lyrics', -80)).toBe('info');
		expect(swipeLayout('info', -80)).toBe('info');
		expect(swipeLayout('info', 80)).toBe('lyrics');
		expect(swipeLayout('art', 80)).toBe('art');
	});
});
