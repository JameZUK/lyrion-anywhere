// Library's remembered choice (chip, album order, grid or list): per device, validated, and
// harmless when storage is blocked or holds something odd.
import { DEFAULT_LIBRARY, LIBRARY_KEY, LIBRARY_SORTS, loadLibraryPrefs, saveLibraryPrefs } from '$lib/library/prefs';

function memory() {
	const m = new Map<string, string>();
	return {
		getItem: (k: string) => m.get(k) ?? null,
		setItem: (k: string, v: string) => void m.set(k, v),
		m,
	};
}

test('a saved choice comes back', () => {
	const s = memory();
	saveLibraryPrefs({ chip: 'artists', sort: 'yearalbum', view: 'list' }, s);
	expect(loadLibraryPrefs(s)).toEqual({ chip: 'artists', sort: 'yearalbum', view: 'list' });
	expect(JSON.parse(s.m.get(LIBRARY_KEY)!)).toEqual({ chip: 'artists', sort: 'yearalbum', view: 'list' });
});

test('nothing saved, or junk, gives the defaults', () => {
	const s = memory();
	expect(loadLibraryPrefs(s)).toEqual(DEFAULT_LIBRARY);
	for (const junk of ['{', 'null', '7', '"x"', '[]', '{"chip":"downloads","sort":"recentlyplayed","view":"carousel"}', '{"chip":{"x":1}}']) {
		s.m.set(LIBRARY_KEY, junk);
		const p = loadLibraryPrefs(s);
		expect(['albums', 'artists', 'playlists', 'favourites']).toContain(p.chip);
		expect(LIBRARY_SORTS).toContain(p.sort);
		expect(['grid', 'list']).toContain(p.view);
	}
	// Downloads is Android's: never a web chip.
	s.m.set(LIBRARY_KEY, '{"chip":"downloads"}');
	expect(loadLibraryPrefs(s).chip).toBe('albums');
	// Random is not one of Library's orders.
	s.m.set(LIBRARY_KEY, '{"sort":"random"}');
	expect(loadLibraryPrefs(s).sort).toBe(DEFAULT_LIBRARY.sort);
});

test('blocked storage never throws', () => {
	const blocked = {
		getItem: () => {
			throw new Error('SecurityError');
		},
		setItem: () => {
			throw new Error('QuotaExceededError');
		},
	};
	expect(loadLibraryPrefs(blocked)).toEqual(DEFAULT_LIBRARY);
	expect(() => saveLibraryPrefs(DEFAULT_LIBRARY, blocked)).not.toThrow();
	expect(loadLibraryPrefs(undefined)).toEqual(DEFAULT_LIBRARY);
});

test('Recently played is not offered while the gateway refuses that sort', () => {
	expect(LIBRARY_SORTS).not.toContain('recentlyplayed');
	expect(LIBRARY_SORTS).toEqual(['new', 'played', 'album', 'artistalbum', 'yearalbum']);
});
