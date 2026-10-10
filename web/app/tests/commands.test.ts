// Command builders: their output must stay accepted by the gateway's allow-list. Every
// builder's output is pinned in tests/fixtures/commands.json, which a Rust test
// (gateway/src/rpc_policy.rs) runs through the real policy. Regenerate the fixture with
// UPDATE_FIXTURES=1 npx vitest run tests/commands.test.ts
import { readFileSync, writeFileSync } from 'node:fs';
import * as c from '$lib/lms/commands';

const FIXTURE = `${__dirname}/fixtures/commands.json`;

// One example of every command the app can send. `player: true` means it targets a player.
const examples = [
	{ player: false, cmd: c.players() },
	{ player: false, cmd: c.albums(0, 100) },
	{ player: false, cmd: c.albums(100, 100, 'new') },
	{ player: false, cmd: c.albums(0, 50, 'random', 'abba') },
	{ player: false, cmd: c.artistAlbums(12) },
	{ player: false, cmd: c.albumById(7) },
	{ player: false, cmd: c.artists(0, 100) },
	{ player: false, cmd: c.artists(0, 100, 'beatles') },
	{ player: false, cmd: c.albumTracks(7) },
	{ player: false, cmd: c.search('abba gold') },
	{ player: true, cmd: c.status() },
	{ player: true, cmd: c.queue(0, 100) },
	{ player: true, cmd: c.play() },
	{ player: true, cmd: c.pause() },
	{ player: true, cmd: c.pause(true) },
	{ player: true, cmd: c.pause(false) },
	{ player: true, cmd: c.stop() },
	{ player: true, cmd: c.skip(1) },
	{ player: true, cmd: c.skip(-1) },
	{ player: true, cmd: c.jump(4) },
	{ player: true, cmd: c.seek(83.27) },
	{ player: true, cmd: c.volume(55) },
	{ player: true, cmd: c.mute() },
	{ player: true, cmd: c.power(true) },
	{ player: true, cmd: c.power(false) },
	{ player: true, cmd: c.removeFromQueue(3) },
	{ player: true, cmd: c.moveInQueue(3, 0) },
	{ player: true, cmd: c.clearQueue() },
	{ player: true, cmd: c.shuffle(1) },
	{ player: true, cmd: c.repeat(2) },
	{ player: true, cmd: c.enqueue({ album: 7 }, 'play') },
	{ player: true, cmd: c.enqueue({ album: 7 }, 'play', 3) },
	{ player: true, cmd: c.enqueue({ album: 7 }, 'next') },
	{ player: true, cmd: c.enqueue({ album: 7 }, 'add') },
	{ player: true, cmd: c.enqueue({ track: 99 }, 'play') },
	{ player: true, cmd: c.enqueue({ track: 99 }, 'next') },
	{ player: true, cmd: c.enqueue({ artist: 12 }, 'add') },
	{ player: true, cmd: c.enqueue({ playlist: 5 }, 'play', 2) },
	{ player: true, cmd: c.enqueue({ playlist: 5 }, 'next') },
	{ player: false, cmd: c.favourites(0, 500) },
	{ player: false, cmd: c.favourites(0, 500, 'f047f379.0') },
	{ player: true, cmd: c.playFavourite('f047f379.3', 'play') },
	{ player: true, cmd: c.playFavourite('f047f379.3', 'next') },
	{ player: true, cmd: c.playFavourite('f047f379.3', 'add') },
	{ player: false, cmd: c.playlists(0, 500) },
	{ player: false, cmd: c.playlistTracks(5) },
	{ player: false, admin: true, cmd: c.newPlaylist('Road trip') },
	{ player: false, admin: true, cmd: c.renamePlaylist(5, 'Road trip 2') },
	{ player: false, admin: true, cmd: c.deletePlaylist(5) },
	{ player: false, admin: true, cmd: c.addToPlaylist(5, 99) },
	{ player: false, admin: true, cmd: c.removeFromPlaylist(5, 3) },
	{ player: false, admin: true, cmd: c.moveInPlaylist(5, 3, 0) },
];

test('builders match the fixture the gateway policy is tested against', () => {
	const json = JSON.stringify(examples, null, 1) + '\n';
	if (process.env.UPDATE_FIXTURES) writeFileSync(FIXTURE, json);
	expect(JSON.parse(readFileSync(FIXTURE, 'utf8'))).toEqual(examples);
});

test('ids must be positive integers', () => {
	expect(c.id(7)).toBe(7);
	expect(c.id('42')).toBe(42);
	for (const bad of [0, -1, 1.5, NaN, '7 ', '7;rescan', 'abc', '', null, undefined, {}, '1'.repeat(20)]) {
		expect(() => c.id(bad)).toThrow(RangeError);
	}
	expect(() => c.albumTracks('1 OR 1=1' as unknown as number)).toThrow(RangeError);
});

test('player ids must look like MAC addresses', () => {
	expect(c.playerId('AA:BB:cc:00:11:22')).toBe('aa:bb:cc:00:11:22');
	for (const bad of ['', 'aa:bb:cc:00:11', 'aa:bb:cc:00:11:22:33', '../x', 'aa-bb-cc-00-11-22']) {
		expect(() => c.playerId(bad)).toThrow(RangeError);
	}
});

test('search text is printable and bounded', () => {
	expect(c.search('a\nb\u0000c')).toEqual(['search', 0, 20, 'term:a b c']);
	const long = c.search('x'.repeat(1000))[3] as string;
	expect(long.length).toBeLessThanOrEqual(205);
});

test('paging is clamped to what the gateway allows', () => {
	expect(c.albums(-5, 10_000)).toEqual(['albums', 0, 500, c.ALBUM_TAGS, 'sort:album']);
	expect(c.queue(0, 0)[2]).toBe(1);
	expect(c.volume(150)).toEqual(['mixer', 'volume', 100]);
	expect(c.volume(-3)).toEqual(['mixer', 'volume', 0]);
	expect(c.seek(-10)).toEqual(['time', 0]);
});

test('enqueue never produces a URL or path form', () => {
	for (const e of examples) {
		for (const arg of e.cmd) expect(String(arg)).not.toMatch(/url:|:\/\/|^\/|file:/);
	}
});

test('cover paths only for real cover ids', () => {
	expect(c.coverPath('1a2b3c4d')).toBe('music/1a2b3c4d/cover_300x300');
	expect(c.coverPath(123, 120)).toBe('music/123/cover_120x120');
	expect(c.coverPath('123', 5000)).toBe('music/123/cover_1000x1000');
	for (const bad of ['', '../../etc', 'http://x/y', '-1', 'zz', '1a2b/../../x', null, 0, -4]) {
		expect(c.coverPath(bad)).toBeNull();
	}
});

test('in-app links only for real ids', () => {
	expect(c.link('album', 7)).toBe('#/album/7');
	expect(c.link('artist', '12,34')).toBe('#/artist/12');
	for (const bad of ['javascript:alert(1)', '"><img src=x>', '', null, -1, '1e3', '../x']) {
		expect(c.link('artist', bad)).toBeNull();
	}
});

test('favourite ids are validated like the gateway does', () => {
	expect(c.itemId('f047f379.3')).toBe('f047f379.3');
	for (const bad of ['', 'a b', 'x;rescan', '../x', 'a/b', 'x'.repeat(129), 7, null]) {
		expect(() => c.itemId(bad)).toThrow(RangeError);
	}
	expect(() => c.playFavourite('url:http://x', 'play')).toThrow(RangeError);
});

test('menu images map only to paths the gateway relays', () => {
	expect(c.imagePath('html/images/favorites.png')).toBe('html/images/favorites.png');
	expect(c.imagePath('/plugins/cache/icons/radiolocal.png')).toBe('plugins/cache/icons/radiolocal.png');
	expect(c.imagePath('plugins/Spotty/html/images/icon.png')).toBe('plugins/Spotty/html/images/icon.png');
	expect(c.imagePath('https://img.example.com/a.jpg', 100)).toBe(
		'imageproxy/https%3A%2F%2Fimg.example.com%2Fa.jpg/image_100x100',
	);
	expect(c.imagePath('/imageproxy/abc/image.png')).toBe('imageproxy/abc/image.png');
	for (const bad of [
		'html/images/../../etc/passwd.png', 'html//images/x.png', 'html/images/x.php', 'javascript:alert(1)',
		'file:///etc/x.png', 'C:\\x.png', 'plugins/../x.png', 'html/images/%2e%2e/x.png', 42, null, 'x'.repeat(3000),
	]) {
		expect(c.imagePath(bad), String(bad)).toBeNull();
	}
});
