// Themes: colour maths, palettes that stay legible for every accent, and safe saved choices.
import { contrast, oklchHex, oklchToSrgb, parseHex, srgbToOklch, toHex } from '$lib/theme/color';
import { ACCENTS, DEFAULT_PREFS, THEMES, applyTheme, loadPrefs, resolveMode, sanitise, savePrefs, toneL, tokens, type ThemePrefs } from '$lib/theme/themes';

test('sRGB and OKLCH convert both ways', () => {
	const red = srgbToOklch(parseHex('#ff0000')!);
	expect(red.l).toBeCloseTo(0.628, 2);
	expect(red.c).toBeCloseTo(0.2577, 2);
	expect(red.h).toBeCloseTo(29.23, 0);
	for (const hex of ['#000000', '#ffffff', '#c2410c', '#1f5fbf', '#808080', '#00ff00']) {
		expect(toHex(oklchToSrgb(srgbToOklch(parseHex(hex)!)))).toBe(hex);
	}
	expect(parseHex('#abc')).toEqual(parseHex('#aabbcc'));
	for (const bad of ['', 'red', '#12345', '#ggg', 'javascript:x']) expect(parseHex(bad)).toBeNull();
});

test('colours outside the screen gamut are brought in, keeping lightness', () => {
	const vivid = oklchToSrgb({ l: 0.9, c: 0.4, h: 260 });
	for (const x of vivid) expect(x).toBeGreaterThanOrEqual(0), expect(x).toBeLessThanOrEqual(1);
	expect(oklchHex({ l: 1.5, c: 0, h: 0 })).toBe('#ffffff');
	expect(oklchHex({ l: -1, c: 0, h: 0 })).toBe('#000000');
	expect(toneL(0)).toBe(0);
	expect(toneL(100)).toBeCloseTo(1, 5);
	expect(toneL(50)).toBeGreaterThan(toneL(40));
});

test('WCAG contrast is computed as specified', () => {
	expect(contrast('#000000', '#ffffff')).toBeCloseTo(21, 5);
	expect(contrast('#ffffff', '#ffffff')).toBeCloseTo(1, 5);
	expect(contrast('#767676', '#ffffff')).toBeCloseTo(4.54, 1);
});

// Every theme, mode and accent (plus awkward ones) must keep text legible (WCAG AA).
const accents = [...ACCENTS.map((a) => a.hex), '#ffff00', '#00ffff', '#000000', '#ffffff', '#7f7f7f'];
for (const theme of THEMES) {
	for (const dark of [false, true]) {
		test(`${theme.name} ${dark ? 'dark' : 'light'} keeps text legible for every accent`, () => {
			for (const accent of theme.accent ? accents : [DEFAULT_PREFS.accent]) {
				const t = tokens({ theme: theme.id, mode: 'auto', accent }, dark);
				const where = `${theme.id} ${dark ? 'dark' : 'light'} ${accent}`;
				for (const surface of ['bg', 'bg-raised']) {
					expect(contrast(t.fg!, t[surface]!), `fg on ${surface}, ${where}`).toBeGreaterThanOrEqual(7);
					expect(contrast(t['fg-muted']!, t[surface]!), `muted on ${surface}, ${where}`).toBeGreaterThanOrEqual(4.5);
					expect(contrast(t.danger!, t[surface]!), `danger on ${surface}, ${where}`).toBeGreaterThanOrEqual(4.5);
				}
				expect(contrast(t['fg-muted']!, t['bg-sunken']!), `muted on sunken, ${where}`).toBeGreaterThanOrEqual(4.5);
				expect(contrast(t['accent-fg']!, t.accent!), `on accent, ${where}`).toBeGreaterThanOrEqual(4.5);
				expect(contrast(t['accent-soft-fg']!, t['accent-soft']!), `on accent-soft, ${where}`).toBeGreaterThanOrEqual(4.5);
				// The accent itself must stand out against the page (links, sliders, focus rings).
				expect(contrast(t.accent!, t.bg!), `accent on bg, ${where}`).toBeGreaterThanOrEqual(3);
				for (const v of Object.values(t)) expect(v).not.toMatch(/[;{}<>]|url\(|expression/i);
			}
		});
	}
}

test('modes resolve against the system setting', () => {
	expect(resolveMode('auto', true)).toBe('dark');
	expect(resolveMode('auto', false)).toBe('light');
	expect(resolveMode('light', true)).toBe('light');
	expect(resolveMode('dark', false)).toBe('dark');
});

test('saved choices are checked; anything odd falls back to defaults', () => {
	expect(sanitise(null)).toEqual(DEFAULT_PREFS);
	expect(sanitise({ theme: 'material', mode: 'dark', accent: '#1F5FBF' })).toEqual({ theme: 'material', mode: 'dark', accent: '#1f5fbf' });
	const hostile = sanitise({ theme: '__proto__', mode: 'x', accent: 'red;background:url(//evil)' });
	expect(hostile).toEqual(DEFAULT_PREFS);
	expect(sanitise({ accent: '#fff' }).accent).toBe(DEFAULT_PREFS.accent); // full form only
});

test('choices are saved and restored, and the old light/dark setting carries over', () => {
	const m = new Map<string, string>();
	const store = { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
	expect(loadPrefs(store)).toEqual(DEFAULT_PREFS);
	m.set('anywhere.theme', 'dark');
	expect(loadPrefs(store)).toEqual({ ...DEFAULT_PREFS, mode: 'dark' });
	const p: ThemePrefs = { theme: 'contrast', mode: 'light', accent: '#00796b' };
	savePrefs(p, store);
	expect(loadPrefs(store)).toEqual(p);
	m.set('anywhere.appearance', '{broken');
	expect(loadPrefs(store)).toEqual(DEFAULT_PREFS);
	const blocked = { getItem: () => { throw new Error('blocked'); }, setItem: () => { throw new Error('blocked'); } };
	expect(loadPrefs(blocked)).toEqual(DEFAULT_PREFS);
	savePrefs(p, blocked);
});

test('applying a theme sets tokens, mode and the browser bar colour', () => {
	applyTheme({ theme: 'material', mode: 'auto', accent: '#1f5fbf' }, true);
	const root = document.documentElement;
	expect(root.dataset.theme).toBe('material');
	expect(root.dataset.mode).toBe('dark');
	expect(root.style.getPropertyValue('--accent')).toMatch(/^#[0-9a-f]{6}$/);
	const meta = document.querySelector('meta[name="theme-color"]') as HTMLMetaElement;
	expect(meta.content).toBe(root.style.getPropertyValue('--bg'));
	applyTheme({ theme: 'classic', mode: 'light', accent: '#1f5fbf' }, true);
	expect(root.dataset.mode).toBe('light');
	expect(root.style.getPropertyValue('--bg')).toBe('#f6f5f2');
	expect(document.querySelectorAll('meta[name="theme-color"]').length).toBe(1);
});
