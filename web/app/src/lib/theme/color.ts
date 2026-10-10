// Colour maths for themes: sRGB ⇄ OKLCH, gamut mapping and WCAG contrast. Palettes are
// generated in OKLCH (perceptually even steps, like Material's tones) and output as sRGB hex,
// which every browser understands.

export interface Oklch {
	l: number; // 0–1
	c: number; // 0–~0.37
	h: number; // degrees
}

const clamp = (x: number, lo = 0, hi = 1) => Math.min(hi, Math.max(lo, x));

/** "#rrggbb" or "#rgb" → [r, g, b] in 0–1, or null. */
export function parseHex(hex: string): [number, number, number] | null {
	const m = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(hex.trim());
	if (!m) return null;
	let s = m[1]!;
	if (s.length === 3) s = s.replace(/./g, (c) => c + c);
	return [0, 2, 4].map((i) => parseInt(s.slice(i, i + 2), 16) / 255) as [number, number, number];
}

export function toHex([r, g, b]: [number, number, number]): string {
	return '#' + [r, g, b].map((x) => Math.round(clamp(x) * 255).toString(16).padStart(2, '0')).join('');
}

const toLinear = (x: number) => (x <= 0.04045 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4);
const fromLinear = (x: number) => (x <= 0.0031308 ? 12.92 * x : 1.055 * x ** (1 / 2.4) - 0.055);

export function srgbToOklch(rgb: [number, number, number]): Oklch {
	const [r, g, b] = rgb.map(toLinear) as [number, number, number];
	const l_ = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
	const m_ = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
	const s_ = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
	const L = 0.2104542553 * l_ + 0.793617785 * m_ - 0.0040720468 * s_;
	const a = 1.9779984951 * l_ - 2.428592205 * m_ + 0.4505937099 * s_;
	const bb = 0.0259040371 * l_ + 0.7827717662 * m_ - 0.808675766 * s_;
	const h = (Math.atan2(bb, a) * 180) / Math.PI;
	return { l: L, c: Math.hypot(a, bb), h: h < 0 ? h + 360 : h };
}

/** Linear-light sRGB (may be out of 0–1 if the colour is outside the gamut). */
function oklchToLinear({ l, c, h }: Oklch): [number, number, number] {
	const a = c * Math.cos((h * Math.PI) / 180);
	const b = c * Math.sin((h * Math.PI) / 180);
	const l_ = (l + 0.3963377774 * a + 0.2158037573 * b) ** 3;
	const m_ = (l - 0.1055613458 * a - 0.0638541728 * b) ** 3;
	const s_ = (l - 0.0894841775 * a - 1.291485548 * b) ** 3;
	return [
		4.0767416621 * l_ - 3.3077115913 * m_ + 0.2309699292 * s_,
		-1.2684380046 * l_ + 2.6097574011 * m_ - 0.3413193965 * s_,
		-0.0041960863 * l_ - 0.7034186147 * m_ + 1.707614701 * s_,
	];
}

const inGamut = (lin: number[]) => lin.every((x) => x >= -1e-4 && x <= 1 + 1e-4);

/** OKLCH → sRGB, lowering chroma (keeping lightness and hue) until it fits the gamut. */
export function oklchToSrgb(c: Oklch): [number, number, number] {
	const l = clamp(c.l);
	let lo = 0;
	let hi = Math.max(0, c.c);
	if (!inGamut(oklchToLinear({ l, c: hi, h: c.h }))) {
		for (let i = 0; i < 24; i++) {
			const mid = (lo + hi) / 2;
			if (inGamut(oklchToLinear({ l, c: mid, h: c.h }))) lo = mid;
			else hi = mid;
		}
		hi = lo;
	}
	return oklchToLinear({ l, c: hi, h: c.h }).map((x) => clamp(fromLinear(clamp(x)))) as [number, number, number];
}

export const oklchHex = (c: Oklch) => toHex(oklchToSrgb(c));

/** WCAG relative luminance of a hex colour. */
export function luminance(hex: string): number {
	const rgb = parseHex(hex);
	if (!rgb) return 0;
	const [r, g, b] = rgb.map(toLinear) as [number, number, number];
	return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG contrast ratio (1–21). */
export function contrast(a: string, b: string): number {
	const [x, y] = [luminance(a), luminance(b)].sort((p, q) => q - p) as [number, number];
	return (x + 0.05) / (y + 0.05);
}
