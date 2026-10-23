// Tappable credits: parsing LMS's per-role fields (tags:AS) into names and links, and that odd
// or hostile answers never throw and never make a link out of something that isn't an id.
import { CREDIT_ORDER, MAX_PEOPLE, nameInRole, parseCredits, roleFromSlug, roleLink, roleSlug } from '#lib/lms/credits';
import * as c from '#lib/lms/commands';

test('every role LMS reports is listed, in order, each name with its id', () => {
	const credits = parseCredits({
		id: 9,
		title: 'Invented Song',
		composer: 'Ada Example, Bo Placeholder',
		composer_ids: '31,32',
		artist: 'The Sample Band',
		artist_ids: '12',
		albumartist: 'The Sample Band',
		albumartist_ids: '12',
		conductor: 'Cy Testcase',
		conductor_ids: 40,
		band: 'Ensemble Fictif',
		band_ids: '41',
		trackartist: 'Di Guest',
		trackartist_ids: '42',
	});
	expect(credits.map((x) => x.role)).toEqual(['ALBUMARTIST', 'ARTIST', 'TRACKARTIST', 'COMPOSER', 'CONDUCTOR', 'BAND']);
	expect(credits.find((x) => x.role === 'COMPOSER')).toEqual({
		role: 'COMPOSER',
		label: 'Composer',
		people: [
			{ name: 'Ada Example', id: 31 },
			{ name: 'Bo Placeholder', id: 32 },
		],
	});
	expect(credits.find((x) => x.role === 'CONDUCTOR')?.people).toEqual([{ name: 'Cy Testcase', id: 40 }]);
	expect(CREDIT_ORDER).toHaveLength(6);
});

test('a single id keeps a name that has a comma in it whole', () => {
	const [cr] = parseCredits({ composer: 'Smith, Jo', composer_ids: '7' });
	expect(cr?.people).toEqual([{ name: 'Smith, Jo', id: 7 }]);
});

test('names and ids that do not pair up are shown as text, without links', () => {
	const [cr] = parseCredits({ composer: 'Smith, Jo, Lee, Al', composer_ids: '7,8,9' });
	expect(cr?.people).toEqual([{ name: 'Smith, Jo, Lee, Al', id: null }]);
	expect(parseCredits({ composer: 'Ada Example' })[0]?.people).toEqual([{ name: 'Ada Example', id: null }]);
	expect(parseCredits({ composer: 'Ada', composer_ids: '0' })[0]?.people).toEqual([{ name: 'Ada', id: null }]);
	expect(parseCredits({ composer: 'Ada', composer_ids: '-3' })[0]?.people).toEqual([{ name: 'Ada', id: null }]);
	expect(parseCredits({ composer: 'Ada', composer_ids: '1;DROP' })[0]?.people).toEqual([{ name: 'Ada', id: null }]);
	expect(parseCredits({ composer: 'Ada', composer_ids: '/srv/music/x' })[0]?.people).toEqual([{ name: 'Ada', id: null }]);
});

test('a very long cast is cut short', () => {
	const n = 200;
	const names = Array.from({ length: n }, (_, i) => `P${i}`).join(', ');
	const ids = Array.from({ length: n }, (_, i) => i + 1).join(',');
	const [cr] = parseCredits({ band: names, band_ids: ids });
	expect(cr?.people.length).toBeLessThanOrEqual(MAX_PEOPLE);
});

test('hostile or odd answers never throw', () => {
	const odd: unknown[] = [
		null,
		undefined,
		0,
		'x',
		[],
		[{ composer: 'A' }],
		{ composer: { a: 1 } },
		{ composer: ['A'], composer_ids: [1] },
		{ composer: '', composer_ids: '1' },
		{ composer: 'A'.repeat(100_000), composer_ids: '1,'.repeat(10_000) },
		{ composer: 'A', composer_ids: '9'.repeat(40) },
		{ composer: '<img src=x onerror=alert(1)>', composer_ids: '5' },
		{ __proto__: { composer: 'X' } },
		Object.create(null),
	];
	for (const v of odd) expect(() => parseCredits(v)).not.toThrow();
	expect(parseCredits(null)).toEqual([]);
	// Markup is just text: the view renders it as text.
	expect(parseCredits({ composer: '<b>x</b>', composer_ids: '5' })[0]?.people[0]?.name).toBe('<b>x</b>');
	// Random fuzz: never throws, ids are always positive integers or null.
	const pick = () => {
		const pool = ['1', '2,3', '', ',', 'a', 1.5, -1, null, {}, [], 'x, y', 'x,y,z', '0', 1e300, NaN];
		return pool[Math.floor(Math.random() * pool.length)];
	};
	for (let i = 0; i < 500; i++) {
		const t = { composer: pick(), composer_ids: pick(), artist: pick(), artist_ids: pick(), band: pick(), band_ids: pick() };
		const out = parseCredits(t);
		for (const cr of out) for (const p of cr.people) expect(p.id === null || (Number.isSafeInteger(p.id) && p.id > 0)).toBe(true);
	}
});

test('role words in links round-trip, and anything else is refused', () => {
	for (const r of c.ROLES) expect(roleFromSlug(roleSlug(r))).toBe(r);
	for (const bad of ['', 'producer', 'COMPOSER;', '../composer', 'composer ', null, 7, 'x'.repeat(50)]) expect(roleFromSlug(bad)).toBeNull();
	expect(roleLink(7, 'COMPOSER')).toBe('#/artist/7/composer');
	expect(roleLink('7', 'BAND')).toBe('#/artist/7/band');
	for (const bad of [0, -1, '1,2', 'x', null, 1.5, '/srv/music']) expect(roleLink(bad, 'COMPOSER')).toBeNull();
});

test('a contributor’s name is read from one of their tracks in that role', () => {
	const r = { titles_loop: [{ composer: 'Ada Example, Bo Placeholder', composer_ids: '31,32' }] };
	expect(nameInRole(r, 32, 'COMPOSER')).toBe('Bo Placeholder');
	expect(nameInRole(r, 99, 'COMPOSER')).toBe('');
	expect(nameInRole(r, 31, 'CONDUCTOR')).toBe('');
	for (const bad of [null, {}, { titles_loop: 'x' }, { titles_loop: [null, 1] }]) expect(nameInRole(bad, 1, 'COMPOSER')).toBe('');
});

test('credit commands stay within the gateway’s grammar', () => {
	expect(c.trackCredits(9)).toEqual(['titles', 0, 1, 'tags:AS', 'track_id:9']);
	expect(c.roleTrack(4, 'COMPOSER')).toEqual(['titles', 0, 1, 'tags:AS', 'artist_id:4', 'role_id:COMPOSER']);
	expect(c.roleAlbumCount(4, 'BAND')).toEqual(['albums', 0, 1, 'artist_id:4', 'role_id:BAND']);
	expect(c.artistAlbums(4, 'CONDUCTOR')).toEqual(['albums', 0, 500, c.ALBUM_TAGS, 'sort:yearalbum', 'artist_id:4', 'role_id:CONDUCTOR']);
	expect(c.artistAlbums(4)).toEqual(['albums', 0, 500, c.ALBUM_TAGS, 'sort:yearalbum', 'artist_id:4']);
	expect(() => c.trackCredits(0)).toThrow(RangeError);
	expect(() => c.roleTrack(4, 'PRODUCER' as c.Role)).toThrow(RangeError);
	expect(() => c.roleAlbumCount(-1, 'BAND')).toThrow(RangeError);
});
