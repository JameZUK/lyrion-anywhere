// The quality badge (docs/plans: Wave 1 §5): format and resolution from LMS's status tags,
// and the Hi-Res rule (more than 16-bit or more than 48 kHz, lossless only). The same
// vectors are in mobile/src/model.rs (the Android app).
import { badgeText, kbpsOf, quality, streamed } from '$lib/lms/quality';

test('lossless: codec bits/kHz, and Hi-Res above CD resolution', () => {
	const cases: [Record<string, unknown>, string, boolean][] = [
		[{ type: 'flc', samplesize: '24', samplerate: '96000' }, 'FLAC 24/96', true],
		[{ type: 'flc', samplesize: '16', samplerate: '44100' }, 'FLAC 16/44.1', false],
		[{ type: 'flc', samplesize: 16, samplerate: 48000 }, 'FLAC 16/48', false],
		[{ type: 'flc', samplesize: '24', samplerate: '44100' }, 'FLAC 24/44.1', true],
		[{ type: 'flc', samplesize: '16', samplerate: '88200' }, 'FLAC 16/88.2', true],
		[{ type: 'alc', samplesize: '24', samplerate: '192000' }, 'ALAC 24/192', true],
		[{ type: 'wav', samplerate: '44100' }, 'WAV 44.1 kHz', false],
		[{ type: 'aif', samplerate: '96000' }, 'AIFF 96 kHz', true],
		[{ type: 'dsf', samplesize: '1', samplerate: '2822400' }, 'DSD64', true],
		[{ type: 'FLC ' }, 'FLAC', false],
	];
	for (const [t, label, hiRes] of cases) {
		const q = quality(t)!;
		expect(q.label, JSON.stringify(t)).toBe(label);
		expect(q.hiRes, JSON.stringify(t)).toBe(hiRes);
		expect(q.lossless).toBe(true);
	}
	expect(badgeText(quality({ type: 'flc', samplesize: '24', samplerate: '96000' })!)).toBe('FLAC 24/96 · Hi-Res');
	expect(badgeText(quality({ type: 'flc', samplesize: '16', samplerate: '44100' })!)).toBe('FLAC 16/44.1');
});

test('lossy: codec and bit rate, never Hi-Res however high the rate', () => {
	expect(badgeText(quality({ type: 'mp3', bitrate: '320kbps', samplerate: '44100' })!)).toBe('MP3 320');
	expect(quality({ type: 'mp3', bitrate: '873kbps VBR' })!.label).toBe('MP3 873');
	expect(quality({ type: 'aac', bitrate: '256000' })!.label).toBe('AAC 256');
	expect(quality({ type: 'ops', bitrate: 128 })!.label).toBe('Opus 128');
	expect(quality({ type: 'ogg' })!.label).toBe('Ogg Vorbis');
	const hiRateLossy = quality({ type: 'aac', samplesize: '24', samplerate: '96000', bitrate: '512kbps' })!;
	expect(hiRateLossy.hiRes).toBe(false);
	expect(hiRateLossy.lossless).toBe(false);
});

test('unknown or hostile input: no badge, or a plain short one', () => {
	for (const bad of [null, undefined, {}, { type: '' }, { type: 42 }, { type: '<script>' }, { type: 'x'.repeat(50) }, { type: 'a b' }, { type: 'flac;rm' }]) {
		expect(quality(bad as never), JSON.stringify(bad)).toBeNull();
	}
	// An unknown but tidy type is shown as itself, never claimed lossless.
	expect(quality({ type: 'xyz', samplesize: '24', samplerate: '96000' })).toMatchObject({ label: 'XYZ', lossless: false, hiRes: false });
	// Absurd numbers are ignored rather than shown.
	expect(quality({ type: 'flc', samplesize: '9999', samplerate: '-1' })!.label).toBe('FLAC');
	expect(quality({ type: 'flc', samplesize: 'NaN', samplerate: '1e9' })!.label).toBe('FLAC');
	expect(quality({ type: 'mp3', bitrate: '<b>320</b>' })!.label).toBe('MP3');
	for (const b of ['', 'fast', 'kbps', '9'.repeat(60), '-320kbps', NaN, Infinity, -5, {}]) expect(kbpsOf(b), String(b)).toBeNull();
});

test('every label is short plain text', () => {
	const types = ['flc', 'alc', 'wav', 'aif', 'ape', 'wvp', 'wmal', 'dsf', 'dff', 'mp3', 'aac', 'ogg', 'ops', 'wma', 'mpc', 'zz9'];
	for (const type of types) {
		for (const samplesize of [undefined, '16', '24', '32']) {
			for (const samplerate of [undefined, '44100', '96000', '352800', '11289600']) {
				const q = quality({ type, samplesize, samplerate, bitrate: '320kbps' });
				expect(q).not.toBeNull();
				expect(badgeText(q!)).toMatch(/^[A-Za-z0-9 ./·-]{1,40}$/);
			}
		}
	}
});

test('the badge says what is streamed to this device: Opus when the gateway re-encoded the track', () => {
	const flac = quality({ type: 'flc', samplesize: '24', samplerate: '96000' });
	const opus = { codec: 'Opus', lossless: false, bits: null };
	expect(badgeText(streamed(flac, opus, 'high')!)).toBe('Opus 160');
	expect(badgeText(streamed(flac, opus, 'low')!)).toBe('Opus 96');
	expect(streamed(flac, opus, 'high')).toMatchObject({ codec: 'Opus', lossless: false, hiRes: false, kbps: 160, rate: 48000 });
	// The level not known: still Opus, never Hi-Res.
	expect(badgeText(streamed(flac, opus, null)!)).toBe('Opus');
	// Not re-encoded: the track's own badge, whatever level was asked for.
	expect(streamed(flac, { codec: 'FLAC', lossless: true, bits: 24 }, 'high')).toBe(flac);
	expect(streamed(flac, null, 'high')).toBe(flac);
	const mp3 = quality({ type: 'mp3', bitrate: '320kbps' });
	expect(streamed(mp3, { codec: 'MP3', lossless: false, bits: null }, 'high')).toBe(mp3);
	// An Opus track in the library streamed as Opus is not the gateway's doing.
	const ops = quality({ type: 'ops', bitrate: 128 });
	expect(streamed(ops, opus, 'high')).toBe(ops);
	// No track details from LMS: Opus at a lower level is the gateway's; at Original it is the track.
	expect(badgeText(streamed(null, opus, 'high')!)).toBe('Opus 160');
	expect(streamed(null, opus, 'original')).toBeNull();
	expect(streamed(null, null, 'high')).toBeNull();
});
