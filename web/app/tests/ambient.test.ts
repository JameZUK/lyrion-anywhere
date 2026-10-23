// Now playing's ambient background (docs/plans: Wave 1 §2) must never cost text contrast: the
// art filter and the scrim keep WCAG AA for every theme, mode and accent over any cover
// (black to white, and anything between), and the mini player's faint art tint leaves its
// text legible too.
import { ART_FILTERS, MINI_TINT_SURFACE, MAX_SCRIM, MIN_SCRIM, filtered, over, scrimAlpha, worstContrast } from '#lib/theme/ambient';
import { ACCENTS, DEFAULT_PREFS, THEMES, tokens } from '#lib/theme/themes';
import { contrast, parseHex, toHex } from '#lib/theme/color';
import { ambientStill } from '#lib/theme/motion';

/** A cover colour as drawn behind the scrim: through the theme's art filter. */
const drawn = (hex: string, dark: boolean) => toHex(parseHex(hex)!.map((x) => filtered(x, dark)) as [number, number, number]);

const accents = [...ACCENTS.map((a) => a.hex), '#ffff00', '#00ffff', '#000000', '#ffffff', '#7f7f7f'];
const covers = ['#000000', '#ffffff', '#ff0000', '#00ff00', '#0000ff', '#ffff00', '#808080', '#3a1f5d', '#f5e6c8'];

for (const theme of THEMES) {
	for (const dark of [false, true]) {
		test(`${theme.name} ${dark ? 'dark' : 'light'}: the scrim keeps AA over any cover`, () => {
			for (const accent of theme.accent ? accents : [DEFAULT_PREFS.accent]) {
				const t = tokens({ theme: theme.id, mode: 'auto', accent }, dark);
				const a = scrimAlpha(t, dark);
				expect(a).toBeGreaterThanOrEqual(MIN_SCRIM);
				expect(a).toBeLessThanOrEqual(MAX_SCRIM);
				expect(t['ambient-scrim']).toBe(`${Math.round(a * 100)}%`);
				expect(t['ambient-filter']).toBe(ART_FILTERS[dark ? 'dark' : 'light'].css);
				const where = `${theme.id} ${dark ? 'dark' : 'light'} ${accent} at ${a}`;
				for (const cover of covers) {
					const under = over(t.bg!, a, drawn(cover, dark));
					expect(contrast(t.fg!, under), `fg, ${cover}, ${where}`).toBeGreaterThanOrEqual(4.5);
					expect(contrast(t['fg-muted']!, under), `muted, ${cover}, ${where}`).toBeGreaterThanOrEqual(4.5);
					expect(contrast(t.accent!, under), `accent, ${cover}, ${where}`).toBeGreaterThanOrEqual(3);
				}
			}
		});

		test(`${theme.name} ${dark ? 'dark' : 'light'}: the mini player's art tint keeps AA`, () => {
			for (const accent of theme.accent ? accents : [DEFAULT_PREFS.accent]) {
				const t = tokens({ theme: theme.id, mode: 'auto', accent }, dark);
				expect(worstContrast(t.fg!, t['bg-raised']!, MINI_TINT_SURFACE), `fg ${accent}`).toBeGreaterThanOrEqual(4.5);
				expect(worstContrast(t['fg-muted']!, t['bg-raised']!, MINI_TINT_SURFACE), `muted ${accent}`).toBeGreaterThanOrEqual(4.5);
			}
		});
	}
}

test('the scrim is as light as it may be, so the art shows', () => {
	for (const dark of [false, true]) {
		const t = tokens(DEFAULT_PREFS, dark);
		const a = scrimAlpha(t, dark);
		const f = ART_FILTERS[dark ? 'dark' : 'light'];
		// The default theme shows a good share of the art.
		expect(a, dark ? 'dark' : 'light').toBeLessThanOrEqual(0.6);
		const range: [number, number] = [f.lo, f.hi];
		if (a > MIN_SCRIM) {
			const b = a - 0.01;
			expect(worstContrast(t['fg-muted']!, t.bg!, b, range) < 4.5 || worstContrast(t.fg!, t.bg!, b, range) < 4.5 || worstContrast(t.accent!, t.bg!, b, range) < 3).toBe(true);
		}
	}
});

test("the art filter's range is what the browser draws (sRGB linear maps, clamped)", () => {
	for (const dark of [false, true]) {
		const f = ART_FILTERS[dark ? 'dark' : 'light'];
		expect(filtered(0, dark)).toBeCloseTo(f.lo);
		expect(filtered(1, dark)).toBeCloseTo(f.hi);
		for (let x = 0; x <= 1; x += 0.05) {
			expect(filtered(x, dark)).toBeGreaterThanOrEqual(f.lo - 1e-9);
			expect(filtered(x, dark)).toBeLessThanOrEqual(f.hi + 1e-9);
		}
		expect(filtered(-3, dark)).toBeCloseTo(f.lo);
		expect(filtered(NaN, dark)).toBeGreaterThanOrEqual(0);
	}
	// As measured in Chromium: brightness(0.5) draws white as #808080.
	expect(Math.round(filtered(1, true) * 255)).toBe(128);
	expect(ART_FILTERS.dark.css).toBe('saturate(1.5) brightness(0.5)');
	expect(ART_FILTERS.light.css).toBe('saturate(1.5) contrast(0.3) brightness(1.6)');
});

test('mixing is clamped and ignores junk', () => {
	expect(over('#ffffff', 1, '#000000')).toBe('#ffffff');
	expect(over('#ffffff', 0, '#000000')).toBe('#000000');
	expect(over('#ffffff', 5, '#000000')).toBe('#ffffff');
	expect(over('#ffffff', -1, '#000000')).toBe('#000000');
	expect(over('red', 0.5, '#000000')).toBe('red');
});

test('the background holds still on low-power devices and to save data', () => {
	expect(ambientStill(undefined)).toBe(false);
	expect(ambientStill({ hardwareConcurrency: 8, deviceMemory: 8 })).toBe(false);
	expect(ambientStill({ hardwareConcurrency: 2 })).toBe(true);
	expect(ambientStill({ deviceMemory: 1 })).toBe(true);
	expect(ambientStill({ hardwareConcurrency: 8, connection: { saveData: true } })).toBe(true);
	expect(ambientStill({ hardwareConcurrency: NaN, deviceMemory: -1 })).toBe(false);
});
