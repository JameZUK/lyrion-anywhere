// Download defaults per network type (SPEC D5), storage estimates, and the offline owner.
import {
	CAP_CHOICES,
	DEFAULT_PREFS,
	MAX_CAP,
	connectionKind,
	reportsNetwork,
	waitForWifi,
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
	expect(DEFAULT_PREFS).toEqual({ wifi: 'original', cellular: 'opus', wifiOnly: true, capBytes: 0 });
	expect(defaultFormat(DEFAULT_PREFS, { connection: { type: 'wifi' } })).toBe('original');
	expect(defaultFormat(DEFAULT_PREFS, { connection: { type: 'cellular' } })).toBe('opus');
	expect(defaultFormat(DEFAULT_PREFS, {})).toBe('original');
	const s = new Mem();
	savePrefs({ wifi: 'mp3', cellular: 'mp3', wifiOnly: false, capBytes: CAP_CHOICES[2] }, s);
	expect(loadPrefs(s)).toEqual({ wifi: 'mp3', cellular: 'mp3', wifiOnly: false, capBytes: 2 * 1024 ** 3 });
	expect(defaultFormat(loadPrefs(s), { connection: { type: 'cellular' } })).toBe('mp3');
});

test('stored preferences that are garbage fall back to the defaults', () => {
	for (const v of [
		'{',
		'null',
		'[]',
		'{"wifi":"flac","cellular":"<script>"}',
		'"x"',
		'{"wifiOnly":"no","capBytes":-5}',
		'{"capBytes":1e300}',
		'{"capBytes":"1073741824"}',
		'{"capBytes":1.5}',
		`{"capBytes":${MAX_CAP + 1}}`,
	]) {
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

test('Wi-Fi only: downloads wait only when the browser says this is mobile data', () => {
	const on = { ...DEFAULT_PREFS, wifiOnly: true };
	const off = { ...DEFAULT_PREFS, wifiOnly: false };
	expect(connectionKind({ connection: { type: 'cellular' } })).toBe('cellular');
	expect(connectionKind({ connection: { type: 'wifi' } })).toBe('wifi');
	expect(connectionKind({ connection: { type: 'ethernet' } })).toBe('wifi');
	expect(connectionKind({ connection: { type: 'none' } })).toBe('unknown');
	// Speed alone (desktop Chrome) or no API (Safari, Firefox) never holds a download back.
	expect(connectionKind({ connection: { effectiveType: '2g', saveData: true } })).toBe('unknown');
	expect(connectionKind({})).toBe('unknown');
	expect(connectionKind(undefined)).toBe('unknown');
	expect(waitForWifi(on, { connection: { type: 'cellular' } })).toBe(true);
	expect(waitForWifi(off, { connection: { type: 'cellular' } })).toBe(false);
	expect(waitForWifi(on, { connection: { type: 'wifi' } })).toBe(false);
	expect(waitForWifi(on, { connection: { effectiveType: 'slow-2g' } })).toBe(false);
	expect(waitForWifi(on, {})).toBe(false);
	expect(reportsNetwork({ connection: { type: 'wifi' } })).toBe(true);
	expect(reportsNetwork({ connection: { effectiveType: '4g' } })).toBe(false);
	expect(reportsNetwork({})).toBe(false);
	expect(reportsNetwork(undefined)).toBe(false);
});

test('storage limits offered: off and whole gigabytes, as on Android', () => {
	expect(CAP_CHOICES[0]).toBe(0);
	for (const c of CAP_CHOICES.slice(1)) expect(c % 1024 ** 3).toBe(0);
	const s = new Mem();
	savePrefs({ ...DEFAULT_PREFS, capBytes: -1 }, s);
	expect(loadPrefs(s).capBytes).toBe(0);
	savePrefs({ ...DEFAULT_PREFS, capBytes: Number.NaN }, s);
	expect(loadPrefs(s).capBytes).toBe(0);
});
