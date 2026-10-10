// Choosing the playback mode from capabilities, platform and preference.
import { chooseMode, detectEnvironment, loadPreference, mediaSource, savePreference, MEDIA_TYPE } from '$lib/player/mode';
import type { Capabilities } from '$lib/player/capabilities';

function caps(o: { mms?: boolean; ms?: boolean; flacManaged?: boolean; flacClassic?: boolean; worklet?: boolean }): Capabilities {
	return {
		managedMediaSource: !!o.mms,
		mediaSource: !!o.ms,
		audioWorklet: o.worklet ?? true,
		audioSession: false,
		types: [{ type: MEDIA_TYPE, managed: !!o.flacManaged, classic: !!o.flacClassic }],
		userAgent: '',
	};
}
const ipad = { appleMobile: true };
const desktop = { appleMobile: false };

describe('platform', () => {
	test('iPadOS (which claims to be a Mac) is recognised by touch support', () => {
		const mac = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 Version/27.0 Safari/605.1.15';
		expect(detectEnvironment({ userAgent: mac, maxTouchPoints: 5 }).appleMobile).toBe(true);
		expect(detectEnvironment({ userAgent: mac, maxTouchPoints: 0 }).appleMobile).toBe(false);
		expect(detectEnvironment({ userAgent: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X)' }).appleMobile).toBe(true);
		expect(detectEnvironment({ userAgent: 'Mozilla/5.0 (Linux; Android 15) Chrome/150.0' })).toEqual({ appleMobile: false, android: true });
	});
});

describe('automatic choice', () => {
	test('iPad with Managed Media Source FLAC: media element, managed', () => {
		expect(chooseMode(caps({ mms: true, flacManaged: true }), ipad)).toMatchObject({ mode: 'media', source: 'managed' });
	});
	test('managed is preferred over classic when both work', () => {
		expect(mediaSource(caps({ mms: true, ms: true, flacManaged: true, flacClassic: true }))).toBe('managed');
		expect(mediaSource(caps({ ms: true, flacClassic: true }))).toBe('classic');
		expect(mediaSource(caps({ mms: true, flacManaged: false }))).toBeNull();
	});
	test('iPad without FLAC Media Source support falls back to Web Audio and says why', () => {
		const c = chooseMode(caps({ mms: true, flacManaged: false }), ipad);
		expect(c.mode).toBe('worklet');
		expect(c.reason).toMatch(/may pause in the background/);
	});
	test('desktops keep Web Audio even when Media Source works', () => {
		expect(chooseMode(caps({ ms: true, flacClassic: true }), desktop).mode).toBe('worklet');
	});
	test('Android uses the media element when it can (background playback), else Web Audio', () => {
		expect(chooseMode(caps({ ms: true, flacClassic: true }), { appleMobile: false, android: true })).toMatchObject({ mode: 'media', source: 'classic' });
		expect(chooseMode(caps({ ms: true }), { appleMobile: false, android: true }).mode).toBe('worklet');
	});
	test('no worklets: media element if possible', () => {
		expect(chooseMode(caps({ ms: true, flacClassic: true, worklet: false }), desktop)).toMatchObject({ mode: 'media', source: 'classic' });
	});
});

describe('preferences', () => {
	test('an explicit choice wins when supported, and falls back with a reason when not', () => {
		expect(chooseMode(caps({ ms: true, flacClassic: true }), desktop, 'media')).toMatchObject({ mode: 'media', source: 'classic' });
		expect(chooseMode(caps({ mms: true, flacManaged: true }), ipad, 'worklet').mode).toBe('worklet');
		const c = chooseMode(caps({}), desktop, 'media');
		expect(c.mode).toBe('worklet');
		expect(c.reason).toMatch(/cannot play the stream that way/);
	});
	test('stored preference is validated', () => {
		const store = (v: string | null) => ({ getItem: () => v });
		expect(loadPreference(store('media'))).toBe('media');
		expect(loadPreference(store('worklet'))).toBe('worklet');
		for (const bad of [null, '', 'auto', 'MEDIA', '<script>']) expect(loadPreference(store(bad))).toBe('auto');
		expect(loadPreference({ getItem: () => { throw new Error('blocked'); } })).toBe('auto');
		const m = new Map<string, string>();
		const s = { setItem: (k: string, v: string) => void m.set(k, v), removeItem: (k: string) => void m.delete(k) };
		savePreference('media', s);
		expect(m.get('anywhere.playback.mode')).toBe('media');
		savePreference('auto', s);
		expect(m.has('anywhere.playback.mode')).toBe(false);
	});
});

describe('timeline', async () => {
	const { Timeline } = await import('$lib/player/media-output');
	test('converts played seconds to frames across rate changes', () => {
		const t = new Timeline();
		t.reset(0, 44_100);
		t.mark(10, 96_000);
		expect(t.frames(0, 1)).toBe(44_100);
		expect(t.frames(9.5, 10.5)).toBe(22_050 + 48_000);
		expect(t.frames(20, 21)).toBe(96_000);
		expect(t.frames(5, 5)).toBe(0);
		expect(t.frames(6, 5)).toBe(0);
		expect(t.frames(NaN, 5)).toBe(0);
	});
	test('a new span replaces anything after it (after a flush)', () => {
		const t = new Timeline();
		t.reset(0, 44_100);
		t.mark(30, 96_000);
		t.mark(12, 48_000); // timeline restarted earlier
		expect(t.frames(12, 13)).toBe(48_000);
		expect(t.frames(31, 32)).toBe(48_000);
	});
	test('no spans: nothing counted', () => {
		expect(new Timeline().frames(0, 10)).toBe(0);
	});
});
