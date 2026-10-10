// The download contract: URLs are built only from a validated id and a format from a fixed
// list, and whatever the server says about a track is checked before it is used or stored.
import { BadInfo, audioType, downloadUrl, infoUrl, parseInfo, pickFormat, trackId } from '$lib/offline/types';
import { contentRange, retryAfter } from '$lib/offline/manager.svelte';

const good = () => ({
	id: 123,
	key: 'a'.repeat(64),
	title: 'Title',
	artist: 'Artist',
	album: 'Album',
	album_id: 7,
	tracknum: 3,
	duration: 241.2,
	original: { format: 'flc', content_type: 'audio/flac', size: 23456789, ext: 'flac' },
	formats: ['original', 'mp3', 'opus'],
});

test('download URLs come only from a positive integer id and a known format', () => {
	expect(infoUrl(123)).toBe('/v1/download/track/123/info');
	expect(downloadUrl(123, 'opus')).toBe('/v1/download/track/123?format=opus');
	for (const bad of [0, -1, 1.5, NaN, Infinity, 2 ** 60, '1/../../admin', '5?format=x', '../1', '1#x', '', null, {}, '0x10', ' 1']) {
		expect(() => downloadUrl(bad as number, 'original'), String(bad)).toThrow(RangeError);
		expect(() => infoUrl(bad as number), String(bad)).toThrow(RangeError);
	}
	for (const f of ['flac', 'original&x=1', '../', 'ORIGINAL', '', 'mp3 ', 'opus#']) {
		expect(() => downloadUrl(1, f as 'mp3'), f).toThrow(RangeError);
	}
	expect(trackId('42')).toBe(42);
});

test('track details are parsed and checked', () => {
	const i = parseInfo(good(), 123);
	expect(i).toMatchObject({ id: 123, key: 'a'.repeat(64), albumId: 7, tracknum: 3, formats: ['original', 'mp3', 'opus'] });
	expect(i.original).toEqual({ format: 'flc', contentType: 'audio/flac', size: 23456789, ext: 'flac' });
});

test('hostile or malformed track details are refused', () => {
	const cases: [string, (g: ReturnType<typeof good>) => unknown][] = [
		['other id', (g) => ({ ...g, id: 124 })],
		['string id', (g) => ({ ...g, id: '123' })],
		['short key', (g) => ({ ...g, key: 'abc' })],
		['path key', (g) => ({ ...g, key: `../../${'a'.repeat(58)}` })],
		['upper-case key', (g) => ({ ...g, key: 'A'.repeat(64) })],
		['no original', (g) => ({ ...g, original: null })],
		['negative size', (g) => ({ ...g, original: { ...g.original, size: -1 } })],
		['string size', (g) => ({ ...g, original: { ...g.original, size: '10' } })],
		['no formats', (g) => ({ ...g, formats: ['flac', 'wav'] })],
		['array', () => []],
		['null', () => null],
	];
	for (const [what, make] of cases) expect(() => parseInfo(make(good()), 123), what).toThrow(BadInfo);
});

test('unsafe strings from the server are neutralised, not trusted', () => {
	const i = parseInfo(
		{
			...good(),
			title: '<img src=x onerror=alert(1)>'.repeat(50),
			album_id: -3,
			original: { format: '<b>flc</b>', content_type: 'text/html', size: 1, ext: '../exe' },
			formats: ['opus', 'evil', 'opus'],
		},
		123,
	);
	expect(i.title.length).toBeLessThanOrEqual(300);
	expect(i.albumId).toBeNull();
	expect(i.original.contentType).toBe('');
	expect(i.original.ext).toBe('');
	expect(i.original.format).toBe('bflcb');
	expect(i.formats).toEqual(['opus']);
	expect(audioType('audio/ogg; codecs=opus')).toBe('audio/ogg; codecs=opus');
	expect(audioType('audio/flac\r\nX-Evil: 1')).toBe('');
});

test('format choice falls back to what the server offers', () => {
	expect(pickFormat('opus', ['original', 'opus'])).toBe('opus');
	expect(pickFormat('opus', ['original'])).toBe('original');
	expect(pickFormat('mp3', ['opus'])).toBe('opus');
});

test('Content-Range and Retry-After are parsed strictly', () => {
	expect(contentRange('bytes 100-199/200')).toEqual({ start: 100, total: 200 });
	expect(contentRange('bytes 100-199/*')).toEqual({ start: 100, total: 0 });
	for (const bad of [null, '', 'bytes=100-', 'bytes -1-2/3', 'items 1-2/3']) expect(contentRange(bad)).toBeNull();
	expect(retryAfter('7')).toBe(7000);
	expect(retryAfter('0')).toBe(1000);
	expect(retryAfter('999999')).toBe(600_000);
	expect(retryAfter(new Date(10_000 + 5000).toUTCString(), 10_000)).toBeGreaterThanOrEqual(1000);
	expect(retryAfter('soon')).toBeUndefined();
	expect(retryAfter(null)).toBeUndefined();
});
