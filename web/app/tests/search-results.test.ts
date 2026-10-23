// Grouped search results and the Top result: shapes from LMS's `search … extended:1` and
// `playlists search:`, with invented names only. Robust on any input.
import { EMPTY_GROUPS, groupResults, normalise, topResult, totalHits } from '#lib/search/results';
import * as c from '#lib/lms/commands';

const answer = {
	contributors_count: 14,
	contributors_loop: [
		{ contributor: 'Placeholder Quartet', contributor_id: 3 },
		{ contributor: 'Placeholders', contributor_id: 4 },
	],
	albums_count: 2,
	albums_loop: [
		{ album: 'Placeholder', album_id: 70, artist: 'Someone Else', artwork_track_id: 'abcdef12' },
		{ album: 'Second Placeholder', album_id: 71 },
	],
	tracks_count: 1,
	tracks_loop: [{ track: 'Placeholder Song', track_id: 900, artist: 'Placeholder Quartet', album: 'Second Placeholder', coverid: 'abcdef12' }],
};
const lists = { count: 1, playlists_loop: [{ id: 5, playlist: 'Placeholder mix' }] };

test('artists, albums, tracks and playlists are grouped with their counts', () => {
	const g = groupResults(answer, lists);
	expect(g.artists.map((h) => h.id)).toEqual([3, 4]);
	expect(g.albums[0]).toEqual({ kind: 'album', id: 70, name: 'Placeholder', sub: 'Someone Else', cover: 'abcdef12' });
	expect(g.tracks[0]).toEqual({ kind: 'track', id: 900, name: 'Placeholder Song', sub: 'Placeholder Quartet · Second Placeholder', cover: 'abcdef12' });
	expect(g.playlists[0]).toEqual({ kind: 'playlist', id: 5, name: 'Placeholder mix', sub: '', cover: null });
	expect(g.counts).toEqual({ artists: 14, albums: 2, tracks: 1, playlists: 1 });
	expect(totalHits(g)).toBe(6);
});

test('the Top result: exact before prefix; artist before album before track', () => {
	const g = groupResults(answer, lists);
	// Exact: the album "Placeholder" beats the artist "Placeholders" (only a prefix match).
	expect(topResult(g, 'placeholder')).toMatchObject({ kind: 'album', id: 70 });
	// Exact artist.
	expect(topResult(g, 'PLACEHOLDERS')).toMatchObject({ kind: 'artist', id: 4 });
	// Prefix only: the first artist that starts with it.
	expect(topResult(g, 'placeh')).toMatchObject({ kind: 'artist', id: 3 });
	// A prefix only a track has.
	expect(topResult(groupResults({ tracks_loop: answer.tracks_loop }), 'placeholder s')).toMatchObject({ kind: 'track' });
	// Nothing close: no Top result.
	expect(topResult(g, 'zzz')).toBeNull();
	expect(topResult(g, '  ')).toBeNull();
	// Accents and case aside.
	expect(normalise('  Ćafé  Ünder ')).toBe('cafe under');
	expect(topResult(groupResults({ contributors_loop: [{ contributor: 'Éclair', contributor_id: 1 }] }), 'eclair')).toMatchObject({ id: 1 });
});

test('rows without a usable id are dropped; counts never fall below what is listed', () => {
	const g = groupResults({
		contributors_count: 'lots',
		contributors_loop: [{ contributor: 'A', contributor_id: '0' }, { contributor: '', contributor_id: 2 }, { contributor: 'B', contributor_id: '3' }, { contributor: 'B again', contributor_id: 3 }, null, 'x', [1]],
		albums_count: -4,
		albums_loop: [{ album: 'X', album_id: 1.5 }, { album: 'Y', album_id: '/srv/music' }, { album: 'Z', album_id: 2, artwork_track_id: '../../etc' }],
	});
	expect(g.artists).toEqual([{ kind: 'artist', id: 3, name: 'B', sub: '', cover: null }]);
	expect(g.counts.artists).toBe(1);
	expect(g.albums).toEqual([{ kind: 'album', id: 2, name: 'Z', sub: '', cover: null }]);
	expect(g.counts.albums).toBe(1);
});

test('hostile or odd answers never throw', () => {
	for (const v of [null, undefined, 1, 'x', [], { contributors_loop: 'x' }, { albums_loop: {} }, { tracks_loop: [null] }, { tracks_loop: new Array(10_000).fill({ track: 'a', track_id: 1 }) }]) {
		expect(() => groupResults(v, v)).not.toThrow();
	}
	expect(groupResults(null)).toEqual(EMPTY_GROUPS);
	for (let i = 0; i < 300; i++) {
		const pool = [null, 1, -1, '2', 'a', {}, [], 'x'.repeat(1000), 1e300];
		const r = () => pool[Math.floor(Math.random() * pool.length)];
		const g = groupResults({ contributors_loop: [{ contributor: r(), contributor_id: r() }], tracks_loop: [{ track: r(), track_id: r(), coverid: r() }] }, { playlists_loop: [{ id: r(), playlist: r() }] });
		for (const h of [...g.artists, ...g.tracks, ...g.playlists]) expect(Number.isSafeInteger(h.id) && h.id > 0).toBe(true);
		expect(() => topResult(g, String(r()))).not.toThrow();
	}
});

test('search commands stay within the gateway’s grammar', () => {
	expect(c.searchAll('blue')).toEqual(['search', 0, 20, 'term:blue', 'extended:1']);
	expect(c.searchTracks('blue', 40, 20)).toEqual(['titles', 40, 20, c.TRACK_TAGS, 'search:blue']);
	expect(c.searchPlaylists('blue', 0, 20)).toEqual(['playlists', 0, 20, 'search:blue']);
	expect(c.albums(20, 20, 'album', 'blue')).toEqual(['albums', 20, 20, c.ALBUM_TAGS, 'sort:album', 'search:blue']);
	expect(c.artists(20, 20, 'blue')).toEqual(['artists', 20, 20, 'search:blue']);
	// Control characters never reach LMS.
	expect(c.searchAll('a\nb')[3]).toBe('term:a b');
});
