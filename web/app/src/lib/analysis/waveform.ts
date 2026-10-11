// Drawing maths for the waveform seek bar: the analysis's buckets reduced to as many bars as
// fit, and the SVG path that draws them. Pure functions, so they are easy to test.

/** Narrowest bar pitch, CSS px (bar plus gap). */
export const BAR_PITCH = 3;
export const MIN_BARS = 16;
export const MAX_BARS = 600;

/** How many bars fit a width (CSS px). */
export function barCount(widthPx: number, pitch = BAR_PITCH): number {
	if (!Number.isFinite(widthPx) || widthPx <= 0 || !(pitch > 0)) return MIN_BARS;
	return Math.min(MAX_BARS, Math.max(MIN_BARS, Math.floor(widthPx / pitch)));
}

/**
 * Bucket values (0–255) reduced to `count` bar heights in 0–1. Each bar is the loudest
 * bucket in its span, so short peaks never vanish. A gentle curve lifts quiet passages so
 * they stay visible without flattening loud ones.
 */
export function bars(values: ArrayLike<number>, count: number): number[] {
	const n = values.length;
	const c = Math.floor(count);
	if (n === 0 || !(c > 0)) return [];
	const out: number[] = new Array(c);
	for (let i = 0; i < c; i++) {
		const a = Math.floor((i * n) / c);
		const b = Math.max(a + 1, Math.floor(((i + 1) * n) / c));
		let m = 0;
		for (let k = a; k < b && k < n; k++) m = Math.max(m, Number(values[k]) || 0);
		out[i] = level(m);
	}
	return out;
}

/** A bucket value (0–255) as a bar height (0–1). */
export function level(v: number): number {
	const x = Math.min(255, Math.max(0, Number.isFinite(v) ? v : 0)) / 255;
	return Math.pow(x, 0.7);
}

const f = (x: number) => (Math.round(x * 100) / 100).toString();

/**
 * An SVG path of bars centred on the middle line, in a `width` × `height` box. Each bar is
 * at least `minHeight` tall (silence still shows as a line) and leaves `gap` to the next.
 */
export function barsPath(levels: number[], width: number, height: number, gap = 1, minHeight = 1): string {
	if (!levels.length || !(width > 0) || !(height > 0)) return '';
	const pitch = width / levels.length;
	const w = Math.max(pitch * 0.4, pitch - gap);
	let d = '';
	for (let i = 0; i < levels.length; i++) {
		const raw = levels[i];
		const l = typeof raw === 'number' && Number.isFinite(raw) ? Math.min(1, Math.max(0, raw)) : 0;
		const h = Math.min(height, Math.max(minHeight, l * height));
		const x = i * pitch;
		const y = (height - h) / 2;
		d += `M${f(x)} ${f(y)}h${f(w)}v${f(h)}h${f(-w)}z`;
	}
	return d;
}

/** The played fraction (0–1) of a track. */
export function fraction(elapsed: number, total: number): number {
	if (!Number.isFinite(elapsed) || !Number.isFinite(total) || total <= 0) return 0;
	return Math.min(1, Math.max(0, elapsed / total));
}
