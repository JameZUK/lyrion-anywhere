// Library's remembered choice (chip, album order, grid or list): per device, validated, and
// harmless when storage is blocked or holds something odd.
import { CHIPS, DEFAULT_LIBRARY, LIBRARY_KEY, LIBRARY_SORTS, loadLibraryPrefs, openSection, saveLibraryPrefs } from '#lib/library/prefs';

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
	for (const junk of ['{', 'null', '7', '"x"', '[]', '{"chip":"downloaded","sort":"recentlyplayed","view":"carousel"}', '{"chip":{"x":1}}', '{"chip":"__proto__"}']) {
		s.m.set(LIBRARY_KEY, junk);
		const p = loadLibraryPrefs(s);
		expect(CHIPS).toContain(p.chip);
		expect(LIBRARY_SORTS).toContain(p.sort);
		expect(['grid', 'list']).toContain(p.view);
	}
	// Wave 3.5: Downloads is one of the web's sections, and Random one of Albums' orders.
	s.m.set(LIBRARY_KEY, '{"chip":"downloads","sort":"random"}');
	expect(loadLibraryPrefs(s)).toMatchObject({ chip: 'downloads', sort: 'random' });
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
	expect(LIBRARY_SORTS).toEqual(['new', 'played', 'album', 'artistalbum', 'yearalbum', 'random']);
});

test('opening a section (what /albums, /artists, /new and /random do) keeps the rest of the choice', () => {
	const s = memory();
	saveLibraryPrefs({ chip: 'genres', sort: 'yearalbum', view: 'list' }, s);
	openSection('albums', 'new', s);
	expect(loadLibraryPrefs(s)).toEqual({ chip: 'albums', sort: 'new', view: 'list' });
	openSection('artists', undefined, s);
	expect(loadLibraryPrefs(s)).toEqual({ chip: 'artists', sort: 'new', view: 'list' });
});
