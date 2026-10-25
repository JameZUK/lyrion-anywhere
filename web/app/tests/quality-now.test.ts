// What the badge and the signal-path diagram are drawn from (docs/plans/signal-path-redesign.md):
// the page's facts put into the shared model's inputs, the worker's served/sent checked again
// on the page, why the level in force is in force, and GET /v1/me's protocol.
import { describe as describeDiagram } from '#lib/player/quality-path';
import { downloadLabel, qualityInputs, type PathFacts } from '#lib/player/quality-now';
import { levelInForce, originOf, sentOf, servedOf } from '#lib/player/stream-quality';
import { playsHere, serverInfoOf } from '#lib/api/me';

const FLAC_96 = { type: 'flc', samplerate: '96000', samplesize: '24', bitrate: '3021kbps' };
const OPUS_SOURCE = { codec: 'Opus', lossless: false, bits: null, channels: 2 };

/** The case reported: a 24/96 FLAC on mobile data, re-encoded to Opus 160 for High. */
function reported(over: Partial<PathFacts> = {}): PathFacts {
	return {
		player: 'local',
		track: FLAC_96,
		source: OPUS_SOURCE,
		served: { codec: 'flac', rate: 96000, bits: 24, channels: 2 },
		sent: { mode: 'opus', kbps: 160, level: 'high' },
		engineRate: 48000,
		deviceRate: 48000,
		origin: originOf('auto', null, { type: 'cellular' }),
		seekable: true,
		...over,
	};
}

test('the reported case: FLAC 24/96, Opus 160 decoded, High on mobile data', () => {
	const d = describeDiagram(qualityInputs(reported()));
	expect(d.badge).toEqual({ verdict: 'transcoded', stream: 'Opus 160', text: 'Transcoded · Opus 160' });
	expect(d.blocks.map((b) => [b.kind, b.format, b.lowers])).toEqual([
		['file', 'FLAC 24/96', false],
		['gateway', 'Opus 160', true],
		['device', 'Opus 48 kHz', false],
	]);
	expect(d.blocks[1]!.note).toBe('Re-encoded to Opus 160 kb/s (High, on mobile data)');
	expect(d.switch).toEqual({ levels: ['original', 'high', 'low'], in_force: 'high', origin_text: 'High, on mobile data', overridden: false, seekable: true });
});

test('passed through: Hi-Res, and the device block says when the browser resamples', () => {
	const d = describeDiagram(
		qualityInputs(
			reported({
				source: { codec: 'FLAC', lossless: true, bits: 24, channels: 2 },
				sent: { mode: 'passthrough' },
				engineRate: 96000,
				deviceRate: 48000,
				origin: originOf('auto', null, { type: 'wifi' }),
			}),
		),
	);
	expect(d.badge!.text).toBe('Hi-Res · FLAC 24/96');
	expect(d.blocks.find((b) => b.kind === 'gateway')!.note).toBe('Passed through unchanged');
	expect(d.blocks.find((b) => b.kind === 'device')).toMatchObject({ format: 'FLAC 24/96', note: 'Resampled to 48 kHz' });
	expect(d.switch!.origin_text).toBe('Original, on Wi-Fi');
});

test('before the stream message: the decoder alone still says Transcoded', () => {
	const d = describeDiagram(qualityInputs(reported({ served: undefined, sent: undefined })));
	expect(d.badge!.text).toBe('Transcoded · Opus');
});

test("a malformed served or sent from the worker is unknown, not shown", () => {
	const d = describeDiagram(qualityInputs(reported({ served: { codec: '<b>FLAC</b>' }, sent: { mode: 'opus', kbps: '160', level: 'high' } })));
	expect(d.blocks.map((b) => b.kind)).toEqual(['file', 'gateway', 'device']);
	expect(d.blocks[1]!.note).toBe('');
});

test('a remote player: File → the player, the badge from the file, no switch', () => {
	const d = describeDiagram(qualityInputs({ player: 'remote', playerName: 'Kitchen', track: FLAC_96, source: OPUS_SOURCE }));
	expect(d.badge!.text).toBe('Hi-Res · FLAC 24/96');
	expect(d.blocks.map((b) => [b.kind, b.title])).toEqual([
		['file', 'File'],
		['player', 'Kitchen'],
	]);
	expect(d.switch).toBeNull();
	const mp3 = describeDiagram(qualityInputs({ player: 'remote', playerName: '', track: { type: 'mp3', bitrate: '320kbps' } }));
	expect(mp3.badge!.text).toBe('Lossy · MP3 320');
	expect(mp3.blocks[0]!.lowers).toBe(true);
	expect(mp3.blocks[1]!.title).toBe('Player');
});

test('offline: Download → This device, with the download format', () => {
	const original = describeDiagram(qualityInputs({ player: 'offline', download: { format: 'original', codec: 'flc' } }));
	expect(original.badge!.text).toBe('Lossless · FLAC');
	expect(original.blocks.map((b) => [b.kind, b.format])).toEqual([
		['download', 'FLAC'],
		['device', 'FLAC'],
	]);
	expect(original.switch).toBeNull();
	const opus = describeDiagram(qualityInputs({ player: 'offline', download: { format: 'opus', codec: 'opus', source: 'flc' } }));
	expect(opus.badge!.text).toBe('Transcoded · Opus 160');
	expect(opus.blocks[0]!.lowers).toBe(true);
	expect(opus.blocks[1]!.format).toBe('Opus 48 kHz');
	// A download stored before its library file's type was kept: what it is, lossy.
	expect(describeDiagram(qualityInputs({ player: 'offline', download: { format: 'opus', codec: 'opus' } })).badge!.text).toBe('Lossy · Opus 160');
	expect(describeDiagram(qualityInputs({ player: 'offline', download: { format: 'mp3', codec: 'mp3', source: 'mp3' } })).badge!.text).toBe('Lossy · MP3 320');
	expect(downloadLabel({ format: 'original', codec: 'zz<' })).toBe('');
	// Nothing playing offline: no badge.
	expect(describeDiagram(qualityInputs({ player: 'offline', download: null })).badge).toBeNull();
});

describe('why the level in force is in force', () => {
	test.each([
		['auto', null, { type: 'cellular' }, 'High, on mobile data'],
		['auto', null, { type: 'wifi' }, 'Original, on Wi-Fi'],
		['auto', null, { type: 'cellular', saveData: true }, 'Data saver, chosen automatically'],
		['auto', null, { type: 'ethernet' }, 'Original, chosen automatically'],
		['low', null, { type: 'cellular' }, 'Data saver, chosen in Settings'],
		['high', 'original', { type: 'cellular' }, 'Original, overridden here'],
		// Safari and Firefox: no Network Information API.
		['auto', null, null, 'Original, chosen automatically'],
		['high', null, null, 'High, chosen in Settings'],
		['low', 'high', null, 'High, overridden here'],
	] as const)('%s, override %s, %j: %s', (pref, override, c, text) => {
		const d = describeDiagram(qualityInputs(reported({ origin: originOf(pref, override, c) })));
		expect(d.switch!.origin_text).toBe(text);
		expect(d.switch!.overridden).toBe(override !== null);
		expect(d.switch!.in_force).toBe(levelInForce(pref, override, c));
	});
});

test('served and sent are checked again on the page', () => {
	expect(servedOf({ codec: 'flac', rate: 96000, bits: 24, channels: 2, gain_db: -3.2 })).toEqual({ codec: 'flac', rate: 96000, bits: 24, channels: 2, gain_db: -3.2 });
	expect(servedOf({ codec: null })).toEqual({ codec: null, rate: null, bits: null, channels: null });
	for (const bad of [null, 'flac', [], { codec: 'FLAC' }, { codec: 'x'.repeat(20) }, { codec: 3 }]) expect(servedOf(bad), JSON.stringify(bad)).toBeNull();
	expect(servedOf({ codec: 'flac', rate: -1, bits: 1.5, gain_db: Infinity })).toEqual({ codec: 'flac', rate: null, bits: null, channels: null });
	expect(sentOf({ mode: 'passthrough', kbps: 5 })).toEqual({ mode: 'passthrough' });
	expect(sentOf({ mode: 'opus', kbps: 96, level: 'low' })).toEqual({ mode: 'opus', kbps: 96, level: 'low' });
	for (const bad of [null, {}, { mode: 'opus' }, { mode: 'opus', kbps: 160, level: 'ultra' }, { mode: 'opus', kbps: 1e9, level: 'high' }, { mode: 'mp3' }]) {
		expect(sentOf(bad), JSON.stringify(bad)).toBeNull();
	}
});

test("GET /v1/me's protocol: a positive whole number, else none (an older gateway)", () => {
	expect(serverInfoOf({ protocol: 2, features: { a: true, b: 1 } })).toEqual({ protocol: 2, features: { a: true } });
	for (const p of [undefined, null, '2', 2.5, 0, -2, true, {}, [2]]) expect(serverInfoOf({ protocol: p }).protocol, JSON.stringify(p)).toBeNull();
	for (const raw of [null, 'x', [], 3]) expect(serverInfoOf(raw)).toEqual({ protocol: null, features: {} });
	expect(playsHere(2)).toBe(true);
	expect(playsHere(3)).toBe(true);
	expect(playsHere(1)).toBe(false);
	expect(playsHere(null)).toBe(false);
});
