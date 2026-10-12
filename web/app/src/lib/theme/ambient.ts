// The ambient background of Now playing: the cover, blurred, behind a scrim of the page
// colour. The scrim is as light as it can be while every text colour on it still meets WCAG
// AA over any cover: the blurred art can be anything from black to white, and (mixing in
// sRGB, as CSS does) the contrast is worst at one of those two ends.
import { contrast, parseHex, toHex } from './color';
import type { Tokens } from './themes';

/** The scrim never goes below this (the art stays a backdrop) or above this (it shows at all). */
export const MIN_SCRIM = 0.6;
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

const EXTREMES = ['#000000', '#ffffff'];

/** The lowest contrast `fg` has on `surface` at `alpha` over any cover. */
export function worstContrast(fg: string, surface: string, alpha: number): number {
	return Math.min(...EXTREMES.map((x) => contrast(fg, over(surface, alpha, x))));
}

/** What must stay legible on the scrim: text (4.5:1) and accents (3:1). */
function legible(t: Tokens, alpha: number): boolean {
	const bg = t.bg!;
	return (
		worstContrast(t.fg!, bg, alpha) >= 4.5 &&
		worstContrast(t['fg-muted']!, bg, alpha) >= 4.5 &&
		worstContrast(t.accent!, bg, alpha) >= 3
	);
}

/** The scrim's opacity for these tokens (0.6–0.96, in steps of 0.01). */
export function scrimAlpha(t: Tokens): number {
	for (let a = MIN_SCRIM; a <= MAX_SCRIM + 1e-9; a += 0.01) {
		const r = Math.round(a * 100) / 100;
		if (legible(t, r)) return r;
	}
	return MAX_SCRIM;
}

/** The scrim as a CSS percentage, for `color-mix(in srgb, var(--bg) <this>, transparent)`. */
export const scrimPercent = (t: Tokens): string => `${Math.round(scrimAlpha(t) * 100)}%`;
