// Now playing's ambient background (docs/plans: Wave 1 §2) must never cost text contrast: the
// scrim keeps WCAG AA for every theme, mode and accent over any cover (black to white), and
// the mini player's faint art tint leaves its text legible too.
import { MINI_TINT_SURFACE, MAX_SCRIM, MIN_SCRIM, over, scrimAlpha, worstContrast } from '$lib/theme/ambient';
import { ACCENTS, DEFAULT_PREFS, THEMES, tokens } from '$lib/theme/themes';
import { contrast } from '$lib/theme/color';
import { ambientStill } from '$lib/theme/motion';

const accents = [...ACCENTS.map((a) => a.hex), '#ffff00', '#00ffff', '#000000', '#ffffff', '#7f7f7f'];
const covers = ['#000000', '#ffffff', '#ff0000', '#00ff00', '#0000ff', '#ffff00', '#808080', '#3a1f5d', '#f5e6c8'];

for (const theme of THEMES) {
	for (const dark of [false, true]) {
		test(`${theme.name} ${dark ? 'dark' : 'light'}: the scrim keeps AA over any cover`, () => {
			for (const accent of theme.accent ? accents : [DEFAULT_PREFS.accent]) {
				const t = tokens({ theme: theme.id, mode: 'auto', accent }, dark);
				const a = scrimAlpha(t);
				expect(a).toBeGreaterThanOrEqual(MIN_SCRIM);
				expect(a).toBeLessThanOrEqual(MAX_SCRIM);
				expect(t['ambient-scrim']).toBe(`${Math.round(a * 100)}%`);
				const where = `${theme.id} ${dark ? 'dark' : 'light'} ${accent} at ${a}`;
				for (const cover of covers) {
					const under = over(t.bg!, a, cover);
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

test('the scrim is as light as it may be (not simply opaque)', () => {
	const t = tokens(DEFAULT_PREFS, true);
	const a = scrimAlpha(t);
	expect(a).toBeLessThan(0.9);
	if (a > MIN_SCRIM) expect(worstContrast(t['fg-muted']!, t.bg!, a - 0.01) < 4.5 || worstContrast(t.fg!, t.bg!, a - 0.01) < 4.5 || worstContrast(t.accent!, t.bg!, a - 0.01) < 3).toBe(true);
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
