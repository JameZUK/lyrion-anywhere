// Download defaults per network type (SPEC D5), storage estimates, and the offline owner.
import {
	DEFAULT_PREFS,
	defaultFormat,
	forgetOwner,
	formatBytes,
	lastOwner,
	loadPrefs,
	networkType,
	rememberOwner,
	requestPersist,
	savePrefs,
	storageInfo,
} from '$lib/offline/settings';

class Mem {
	m = new Map<string, string>();
	getItem = (k: string) => this.m.get(k) ?? null;
	setItem = (k: string, v: string) => void this.m.set(k, v);
	removeItem = (k: string) => void this.m.delete(k);
}

test('the network type comes from the Network Information API, else Wi-Fi', () => {
	expect(networkType({})).toBe('wifi');
	expect(networkType(undefined)).toBe('wifi');
	expect(networkType({ connection: { type: 'cellular' } })).toBe('cellular');
	expect(networkType({ connection: { type: 'wifi' } })).toBe('wifi');
	expect(networkType({ connection: { type: 'ethernet', effectiveType: '2g' } })).toBe('wifi');
	expect(networkType({ connection: { effectiveType: '4g' } })).toBe('wifi');
	expect(networkType({ connection: { effectiveType: '3g' } })).toBe('cellular');
	expect(networkType({ connection: { type: 'unknown', effectiveType: 'slow-2g' } })).toBe('cellular');
	expect(networkType({ connection: { saveData: true } })).toBe('cellular');
});

test('defaults: Original on Wi-Fi, Opus on mobile data; the user can change them', () => {
	expect(DEFAULT_PREFS).toEqual({ wifi: 'original', cellular: 'opus' });
	expect(defaultFormat(DEFAULT_PREFS, { connection: { type: 'wifi' } })).toBe('original');
	expect(defaultFormat(DEFAULT_PREFS, { connection: { type: 'cellular' } })).toBe('opus');
	expect(defaultFormat(DEFAULT_PREFS, {})).toBe('original');
	const s = new Mem();
	savePrefs({ wifi: 'mp3', cellular: 'mp3' }, s);
	expect(loadPrefs(s)).toEqual({ wifi: 'mp3', cellular: 'mp3' });
	expect(defaultFormat(loadPrefs(s), { connection: { type: 'cellular' } })).toBe('mp3');
});

test('stored preferences that are garbage fall back to the defaults', () => {
	for (const v of ['{', 'null', '[]', '{"wifi":"flac","cellular":"<script>"}', '"x"']) {
		const s = new Mem();
		s.setItem('anywhere.downloads', v);
		expect(loadPrefs(s), v).toEqual(DEFAULT_PREFS);
	}
	expect(loadPrefs({ getItem: () => { throw new Error('blocked'); } })).toEqual(DEFAULT_PREFS);
});

test('storage use and persistence are reported, and failures are tolerated', async () => {
	expect(await storageInfo({ estimate: async () => ({ usage: 5, quota: 100 }), persisted: async () => false })).toEqual({ usage: 5, quota: 100, persisted: false });
	expect(await storageInfo(undefined)).toEqual({});
	expect(await storageInfo({ estimate: () => Promise.reject(new Error('x')) })).toEqual({});
	const persist = vi.fn(async () => true);
	expect(await requestPersist({ persisted: async () => false, persist })).toBe(true);
	expect(persist).toHaveBeenCalledOnce();
	expect(await requestPersist({ persisted: async () => true, persist })).toBe(true);
	expect(persist).toHaveBeenCalledOnce();
	expect(await requestPersist(undefined)).toBe(false);
	expect(formatBytes(23_456_789)).toBe('22.4 MB');
	expect(formatBytes(3 * 1024 ** 3)).toBe('3.00 GB');
	expect(formatBytes(-1)).toBe('0 MB');
});

test('the offline owner is remembered and forgotten', () => {
	const s = new Mem();
	expect(lastOwner(s)).toBeNull();
	rememberOwner('alice', s);
	expect(lastOwner(s)).toBe('alice');
	forgetOwner(s);
	expect(lastOwner(s)).toBeNull();
	s.setItem('anywhere.offlineUser', 'x'.repeat(500));
	expect(lastOwner(s)).toBeNull();
});
