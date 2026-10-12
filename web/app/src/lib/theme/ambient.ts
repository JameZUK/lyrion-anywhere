// The ambient background of Now playing: the cover, blurred, behind a scrim of the page
// colour. Two things keep every text colour on it at WCAG AA over any cover:
// - the art's own range is bounded by CSS filters (`brightness` and `contrast` are linear
//   maps in sRGB in browsers): at most half bright in dark mode, washed light in light mode;
// - the scrim is then as light as it can be while the worst case still passes. Mixing in
//   sRGB (as CSS does), the contrast is worst at one end of the art's range.
import { contrast, parseHex, toHex } from './color';
import type { Tokens } from './themes';

/** The scrim never goes below this (the art stays a backdrop) or above this (it shows at all). */
export const MIN_SCRIM = 0.3;
export const MAX_SCRIM = 0.96;

/** The mini player's surface over its faint art tint: at least this opaque (the tint is the rest). */
export const MINI_TINT_SURFACE = 0.94;

/** `top` at `alpha` over the opaque `under` (sRGB mixing, like CSS color-mix). */
export function over(top: string, alpha: number, under: string): string {
	const a = parseHex(top);
	const b = parseHex(under);
	if (!a || !b) return top;
	const k = Math.min(1, Math.max(0, alpha));
	return toHex([0, 1, 2].map((i) => a[i]! * k + b[i]! * (1 - k)) as [number, number, number]);
}

/** How the art is filtered: `saturate` first (its output is clamped to 0–1), then
 *  `contrast` and `brightness`, linear maps (in sRGB) that bound the range. */
export interface ArtFilter {
	saturate: number;
	contrast: number;
	brightness: number;
	css: string;
	/** The range (per sRGB channel, 0–1) any art ends up in. */
	lo: number;
	hi: number;
}

function artFilter(saturate: number, c: number, b: number): ArtFilter {
	const css = [`saturate(${saturate})`, c !== 1 ? `contrast(${c})` : '', `brightness(${b})`].filter(Boolean).join(' ');
	const f = (x: number) => Math.min(1, Math.max(0, ((x - 0.5) * c + 0.5) * b));
	return { saturate, contrast: c, brightness: b, css, lo: f(0), hi: f(1) };
}

/** Dark: at most half bright. Light: washed out and lifted. */
export const ART_FILTERS: Record<'light' | 'dark', ArtFilter> = {
	dark: artFilter(1.5, 1, 0.5),
	light: artFilter(1.5, 0.3, 1.6),
};

/** A channel value (0–1, after `saturate`) as the browser draws it once filtered. */
export function filtered(x: number, dark: boolean): number {
	const f = ART_FILTERS[dark ? 'dark' : 'light'];
	const v = Number.isFinite(x) ? Math.min(1, Math.max(0, x)) : 0;
	return Math.min(1, Math.max(0, ((v - 0.5) * f.contrast + 0.5) * f.brightness));
}

const grey = (v: number) => toHex([v, v, v]);
const EXTREMES = ['#000000', '#ffffff'];

/** The lowest contrast `fg` has on `surface` at `alpha` over any (unfiltered) art. */
export function worstContrast(fg: string, surface: string, alpha: number, range: [number, number] = [0, 1]): number {
	const ends = range[0] === 0 && range[1] === 1 ? EXTREMES : range.map(grey);
	return Math.min(...ends.map((x) => contrast(fg, over(surface, alpha, x))));
}

/** What must stay legible on the scrim: text (4.5:1) and accents (3:1). */
function legible(t: Tokens, alpha: number, range: [number, number]): boolean {
	const bg = t.bg!;
	return (
		worstContrast(t.fg!, bg, alpha, range) >= 4.5 &&
		worstContrast(t['fg-muted']!, bg, alpha, range) >= 4.5 &&
		worstContrast(t.accent!, bg, alpha, range) >= 3
	);
}

/** The scrim's opacity for these tokens over the filtered art (0.3–0.96, steps of 0.01). */
export function scrimAlpha(t: Tokens, dark: boolean): number {
	const f = ART_FILTERS[dark ? 'dark' : 'light'];
	for (let a = MIN_SCRIM; a <= MAX_SCRIM + 1e-9; a += 0.01) {
		const r = Math.round(a * 100) / 100;
		if (legible(t, r, [f.lo, f.hi])) return r;
	}
	return MAX_SCRIM;
}

/** The scrim as a CSS percentage, for `color-mix(in srgb, var(--bg) <this>, transparent)`. */
export const scrimPercent = (t: Tokens, dark: boolean): string => `${Math.round(scrimAlpha(t, dark) * 100)}%`;
