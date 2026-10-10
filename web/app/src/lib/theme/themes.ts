// Themes: each turns the user's choices (theme, light/dark mode, accent colour) into the
// design tokens the app's CSS uses (--bg, --fg, --accent, …). Applied at start-up, and again
// whenever the choice or the system's light/dark setting changes.
//
// Material (the default) follows Material Design 3: colour roles from tonal palettes built
// around one seed colour, tonal surfaces instead of borders, large rounded shapes.
import { type Oklch, oklchHex, parseHex, srgbToOklch } from './color';

export type ThemeId = 'material' | 'classic' | 'contrast';
export type Mode = 'auto' | 'light' | 'dark';

export interface ThemePrefs {
	theme: ThemeId;
	mode: Mode;
	/** Seed colour (#rrggbb) for themes that take one. */
	accent: string;
}

export interface ThemeInfo {
	id: ThemeId;
	name: string;
	description: string;
	/** Whether the accent colour can be chosen. */
	accent: boolean;
}

export const THEMES: ThemeInfo[] = [
	{ id: 'material', name: 'Material', description: 'Material Design: tonal colours from your accent, soft shapes.', accent: true },
	{ id: 'classic', name: 'Classic', description: 'The original warm look, with outlined cards.', accent: false },
	{ id: 'contrast', name: 'High contrast', description: 'Maximum legibility: black and white, strong outlines.', accent: false },
];

/** Accent colours offered (any colour can be picked too). */
export const ACCENTS: { name: string; hex: string }[] = [
	{ name: 'Ember', hex: '#c2410c' },
	{ name: 'Violet', hex: '#6750a4' },
	{ name: 'Blue', hex: '#1f5fbf' },
	{ name: 'Teal', hex: '#00796b' },
	{ name: 'Green', hex: '#2e7d32' },
	{ name: 'Rose', hex: '#b4235a' },
	{ name: 'Red', hex: '#b3261e' },
	{ name: 'Slate', hex: '#4a5d6e' },
];

export const DEFAULT_PREFS: ThemePrefs = { theme: 'material', mode: 'auto', accent: ACCENTS[0]!.hex };

export type Tokens = Record<string, string>;

/** OKLab lightness of a grey with CIELAB lightness `tone` (Material's tone scale, 0–100). */
export function toneL(tone: number): number {
	const t = Math.min(100, Math.max(0, tone));
	const y = t > 8 ? ((t + 16) / 116) ** 3 : t / 903.3;
	return Math.cbrt(y);
}

/** Material colour roles for a seed, in light or dark. */
function material(seed: string, dark: boolean): Tokens {
	const s = srgbToOklch(parseHex(seed) ?? parseHex(DEFAULT_PREFS.accent)!);
	const hue = s.h;
	const chroma = Math.min(0.2, Math.max(0.08, s.c));
	const tone = (t: number, c: number, h = hue) => oklchHex({ l: toneL(t), c, h } satisfies Oklch);
	const P = (t: number) => tone(t, chroma);
	const N = (t: number) => tone(t, 0.008); // neutral, faintly tinted by the seed
	const NV = (t: number) => tone(t, 0.02); // neutral variant
	const E = (t: number) => tone(t, 0.17, 27); // error
	return dark
		? {
				bg: N(6),
				'bg-raised': N(12),
				'bg-sunken': N(20),
				fg: N(90),
				'fg-muted': NV(78),
				line: NV(30),
				accent: P(80),
				'accent-fg': P(20),
				'accent-soft': P(30),
				'accent-soft-fg': P(90),
				danger: E(80),
				shadow: '0 1px 3px rgb(0 0 0 / 30%), 0 4px 12px rgb(0 0 0 / 25%)',
			}
		: {
				bg: N(97),
				'bg-raised': N(99),
				'bg-sunken': N(91),
				fg: N(10),
				'fg-muted': NV(32),
				line: NV(85),
				accent: P(40),
				'accent-fg': P(100),
				'accent-soft': P(90),
				'accent-soft-fg': P(10),
				danger: E(40),
				shadow: '0 1px 2px rgb(0 0 0 / 8%), 0 3px 10px rgb(0 0 0 / 6%)',
			};
}

const CLASSIC = {
	light: {
		bg: '#f6f5f2',
		'bg-raised': '#ffffff',
		'bg-sunken': '#ebe9e4',
		fg: '#1c1b19',
		'fg-muted': '#6b6862',
		line: '#dcd9d2',
		accent: '#c2410c',
		'accent-fg': '#ffffff',
		'accent-soft': '#fde7da',
		'accent-soft-fg': '#1c1b19',
		danger: '#b91c1c',
		shadow: '0 1px 2px rgb(0 0 0 / 6%), 0 8px 24px rgb(0 0 0 / 8%)',
	},
	dark: {
		bg: '#121212',
		'bg-raised': '#1c1c1c',
		'bg-sunken': '#0b0b0b',
		fg: '#f2f1ee',
		'fg-muted': '#a39f97',
		line: '#2e2d2b',
		accent: '#fb923c',
		'accent-fg': '#1a0d05',
		'accent-soft': '#3a2316',
		'accent-soft-fg': '#f2f1ee',
		danger: '#f87171',
		shadow: '0 1px 2px rgb(0 0 0 / 40%), 0 8px 24px rgb(0 0 0 / 40%)',
	},
};

const CONTRAST = {
	light: {
		bg: '#ffffff',
		'bg-raised': '#ffffff',
		'bg-sunken': '#e6e6e6',
		fg: '#000000',
		'fg-muted': '#262626',
		line: '#000000',
		accent: '#0037a6',
		'accent-fg': '#ffffff',
		'accent-soft': '#d6e2ff',
		'accent-soft-fg': '#000000',
		danger: '#9b0000',
		shadow: 'none',
	},
	dark: {
		bg: '#000000',
		'bg-raised': '#000000',
		'bg-sunken': '#262626',
		fg: '#ffffff',
		'fg-muted': '#e6e6e6',
		line: '#ffffff',
		accent: '#ffd60a',
		'accent-fg': '#000000',
		'accent-soft': '#3d3300',
		'accent-soft-fg': '#ffffff',
		danger: '#ff8a8a',
		shadow: 'none',
	},
};

const SHAPES: Record<ThemeId, Tokens> = {
	material: { radius: '16px', 'radius-sm': '12px', font: "Roboto, 'Google Sans', system-ui, -apple-system, 'Segoe UI', sans-serif" },
	classic: { radius: '12px', 'radius-sm': '8px', font: "system-ui, -apple-system, 'Segoe UI', Roboto, 'Helvetica Neue', sans-serif" },
	contrast: { radius: '6px', 'radius-sm': '4px', font: "system-ui, -apple-system, 'Segoe UI', Roboto, 'Helvetica Neue', sans-serif" },
};

/** Light or dark, given the mode and the system setting. */
export const resolveMode = (mode: Mode, systemDark: boolean): 'light' | 'dark' =>
	mode === 'auto' ? (systemDark ? 'dark' : 'light') : mode;

/** All design tokens for these choices. */
export function tokens(p: ThemePrefs, dark: boolean): Tokens {
	const colours =
		p.theme === 'material' ? material(p.accent, dark) : (p.theme === 'contrast' ? CONTRAST : CLASSIC)[dark ? 'dark' : 'light'];
	return { ...colours, ...SHAPES[p.theme] };
}

const HEX = /^#[0-9a-f]{6}$/i;

/** Saved choices, checked (anything odd falls back to the defaults). */
export function sanitise(v: unknown): ThemePrefs {
	const o = (v && typeof v === 'object' ? v : {}) as Record<string, unknown>;
	return {
		theme: THEMES.some((t) => t.id === o.theme) ? (o.theme as ThemeId) : DEFAULT_PREFS.theme,
		mode: o.mode === 'light' || o.mode === 'dark' || o.mode === 'auto' ? o.mode : DEFAULT_PREFS.mode,
		accent: typeof o.accent === 'string' && HEX.test(o.accent) ? o.accent.toLowerCase() : DEFAULT_PREFS.accent,
	};
}

const KEY = 'anywhere.appearance';
const OLD_KEY = 'anywhere.theme'; // before themes: 'system' | 'light' | 'dark'

type Store = Pick<Storage, 'getItem' | 'setItem'>;

function safeLocal(): Store | undefined {
	try {
		return globalThis.localStorage;
	} catch {
		return undefined;
	}
}

export function loadPrefs(store: Store | undefined = safeLocal()): ThemePrefs {
	try {
		const saved = store?.getItem(KEY);
		if (saved) return sanitise(JSON.parse(saved));
		const old = store?.getItem(OLD_KEY);
		if (old === 'light' || old === 'dark') return { ...DEFAULT_PREFS, mode: old };
	} catch {
		/* unreadable: defaults */
	}
	return { ...DEFAULT_PREFS };
}

export function savePrefs(p: ThemePrefs, store: Store | undefined = safeLocal()) {
	try {
		store?.setItem(KEY, JSON.stringify(sanitise(p)));
	} catch {
		/* not essential */
	}
}

/** Put the tokens on the page: CSS variables, data attributes, and the browser bar colour. */
export function applyTheme(p: ThemePrefs, systemDark: boolean, doc: Document = document) {
	const mode = resolveMode(p.mode, systemDark);
	const root = doc.documentElement;
	for (const [k, v] of Object.entries(tokens(p, mode === 'dark'))) root.style.setProperty(`--${k}`, v);
	root.dataset.theme = p.theme;
	root.dataset.mode = mode;
	root.style.colorScheme = mode;
	let meta = doc.querySelector<HTMLMetaElement>('meta[name="theme-color"]');
	if (!meta) {
		meta = doc.createElement('meta');
		meta.name = 'theme-color';
		doc.head.append(meta);
	}
	meta.content = tokens(p, mode === 'dark').bg!;
}
