// The web signal path (docs/plans: Wave 1 §5; docs/plans/streaming-quality.md): what it says
// for this browser's own player, honestly, including the stream that actually arrives (Opus
// from the gateway, or the track as stored), and how little it claims for other players.
import { quality } from '#lib/lms/quality';
import { signalPath, type SignalInput } from '#lib/player/signal-path';

const flac: SignalInput['stream'] = { codec: 'FLAC', lossless: true, bits: 24 };
const opus: SignalInput['stream'] = { codec: 'Opus', lossless: false, bits: null };
const mp3: SignalInput['stream'] = { codec: 'MP3', lossless: false, bits: null };

const base: SignalInput = {
	quality: quality({ type: 'flc', samplesize: '24', samplerate: '96000' }),
	thisDevice: true,
	playerName: 'This browser',
	mode: { mode: 'worklet', source: null, reason: '' },
	volumeFollows: true,
	gain: 1,
	engineRate: 96000,
	deviceRate: 96000,
	stream: flac,
	level: 'original',
};
const value = (v: ReturnType<typeof signalPath>, label: string) => v.rows.find((r) => r.label === label)?.value;

test('lossless at the device rate and full volume: lossless to the browser', () => {
	const v = signalPath(base);
	expect(v.verdict).toBe('lossless');
	expect(v.headline).toBe('Lossless to the browser');
	expect(value(v, 'Track in the library')).toBe('FLAC 24/96 · Hi-Res · lossless');
	expect(value(v, 'Stream to this device')).toBe('FLAC 24-bit, as stored (not re-encoded)');
	expect(value(v, 'Decoding')).toMatch(/^On this device/);
	expect(value(v, 'Output')).toBe('Web Audio (AudioWorklet)');
	expect(value(v, 'Rate')).toMatch(/^96 kHz, the output device's own rate/);
	// It never claims more than the browser: the system may still change the sound.
	expect(v.notes.join(' ')).toMatch(/operating system/);
	expect(v.notes.join(' ')).not.toMatch(/Settings/);
});

test('an Opus stream from the gateway is named with its bit rate, and never called lossless', () => {
	// The forum case: a 96 kHz FLAC in the library, High on an iPhone. The gateway sends Opus
	// (always 48 kHz) and Safari plays it through a media element fed losslessly as FLAC in
	// MP4, which read as "96 kHz FLAC converted to 48 kHz MP4" when the panel said no more.
	const v = signalPath({
		...base,
		stream: opus,
		level: 'high',
		engineRate: 48000,
		deviceRate: 48000,
		mode: { mode: 'media', source: 'managed', mime: 'audio/mp4; codecs="fLaC"', reason: '' },
	});
	expect(v.verdict).toBe('stream');
	expect(v.headline).toBe('Re-encoded to Opus 160 kb/s');
	expect(value(v, 'Track in the library')).toBe('FLAC 24/96 · Hi-Res · lossless');
	expect(value(v, 'Stream to this device')).toBe('Opus 160 kb/s, re-encoded by the gateway for the High setting');
	expect(value(v, 'Output')).toBe('Media element (the decoded audio, re-wrapped losslessly as FLAC in MP4)');
	expect(value(v, 'Rate')).toBe("48 kHz, the output device's own rate (Opus is always 48 kHz)");
	expect(v.notes.join(' ')).toMatch(/Streaming quality to Original in Settings › Playback/);
	// Data saver, on a device that resamples.
	const low = signalPath({ ...base, stream: opus, level: 'low', engineRate: 48000, deviceRate: 44100 });
	expect(low.headline).toBe('Re-encoded to Opus 96 kb/s');
	expect(value(low, 'Stream to this device')).toBe('Opus 96 kb/s, re-encoded by the gateway for the Data saver setting');
	expect(value(low, 'Rate')).toBe('48 kHz → 44.1 kHz, resampled by the browser (Opus is always 48 kHz)');
	// The engine says Opus for a FLAC track whatever level was asked for: its word wins.
	const odd = signalPath({ ...base, stream: opus, level: 'original', engineRate: 48000 });
	expect(odd.verdict).toBe('stream');
	expect(value(odd, 'Stream to this device')).toBe('Opus, re-encoded by the gateway');
	expect(odd.headline).toBe('Re-encoded to Opus');
});

test('a lossy track streamed as stored is lossy, not re-encoded; an Opus track is just Opus', () => {
	const asMp3 = signalPath({ ...base, quality: quality({ type: 'mp3', bitrate: '320kbps' }), stream: mp3, level: 'high' });
	expect(asMp3.verdict).toBe('lossy');
	expect(asMp3.headline).toBe('Lossy source');
	expect(value(asMp3, 'Stream to this device')).toBe('MP3, as stored (not re-encoded)');
	const asOpus = signalPath({ ...base, quality: quality({ type: 'ops', bitrate: 128 }), stream: opus, level: 'high' });
	expect(asOpus.verdict).toBe('lossy');
	expect(value(asOpus, 'Stream to this device')).toBe('Opus, as stored (not re-encoded)');
	// The engine knows the stream even when LMS gave no type for the track.
	expect(signalPath({ ...base, quality: null, stream: opus, level: 'high' }).verdict).toBe('stream');
	expect(signalPath({ ...base, quality: null, stream: opus, level: 'original' }).verdict).toBe('lossy');
	expect(signalPath({ ...base, quality: null, stream: flac }).verdict).toBe('lossless');
	expect(value(signalPath({ ...base, quality: null, stream: { codec: 'PCM', lossless: true, bits: 16 } }), 'Stream to this device')).toBe('PCM 16-bit, as stored (not re-encoded)');
});

test('before the first audio is decoded, nothing is claimed that is not known', () => {
	const v = signalPath({ ...base, stream: null });
	expect(value(v, 'Stream to this device')).toBe('Not known yet');
	// At Original the stream can only be the track as stored.
	expect(v.verdict).toBe('lossless');
	// At a lower level it may or may not have been re-encoded yet: no verdict.
	const high = signalPath({ ...base, stream: null, level: 'high' });
	expect(high.verdict).toBe('unknown');
	expect(high.headline).toBe('Not known yet');
	expect(signalPath({ ...base, quality: null, stream: null }).verdict).toBe('unknown');
});

test('resampled, volume applied, media element and lossy sources are said plainly', () => {
	expect(signalPath({ ...base, deviceRate: 48000 }).verdict).toBe('resampled');
	expect(value(signalPath({ ...base, deviceRate: 48000 }), 'Rate')).toBe('96 kHz → 48 kHz, resampled by the browser');
	const quiet = signalPath({ ...base, gain: 0.5 });
	expect(quiet.verdict).toBe('volume');
	expect(value(quiet, 'Volume and DSP')).toMatch(/50% gain/);
	expect(value(signalPath({ ...base, volumeFollows: false }), 'Volume and DSP')).toMatch(/buttons/);
	expect(value(signalPath({ ...base, mode: { mode: 'media', source: 'managed', mime: 'audio/mp4; codecs="alac"', reason: '' } }), 'Output')).toBe(
		'Media element (the decoded audio, re-wrapped losslessly as Apple Lossless in MP4)',
	);
	expect(signalPath({ ...base, quality: quality({ type: 'mp3', bitrate: '320kbps' }), stream: mp3 }).verdict).toBe('lossy');
	// Unknown device rate: the rate alone, no resampling claim either way.
	expect(value(signalPath({ ...base, deviceRate: null }), 'Rate')).toBe('96 kHz');
	expect(value(signalPath({ ...base, mode: null }), 'Output')).toBe('Not playing here');
});

test('another player: only the track is known', () => {
	const v = signalPath({ ...base, thisDevice: false, playerName: 'Kitchen', stream: null, level: null });
	expect(value(v, 'Track in the library')).toBe('FLAC 24/96 · Hi-Res · lossless');
	expect(value(v, 'Decoding')).toBe('On Kitchen');
	expect(v.rows.some((r) => r.label === 'Rate' || r.label === 'Stream to this device')).toBe(false);
	expect(v.verdict).toBe('source');
	expect(v.headline).toBe('Lossless source');
	expect(signalPath({ ...base, thisDevice: false, quality: null, stream: null }).verdict).toBe('unknown');
	expect(signalPath({ ...base, thisDevice: false, quality: quality({ type: 'mp3' }), stream: null }).headline).toBe('Lossy source');
});

test('junk gains never show nonsense', () => {
	for (const gain of [NaN, Infinity, -1, 7]) {
		expect(value(signalPath({ ...base, gain }), 'Volume and DSP')).not.toMatch(/NaN|Infinity|-\d|\d{4,}%/);
	}
});

test('every row and headline is short plain text', () => {
	const streams: SignalInput['stream'][] = [flac, opus, mp3, null, { codec: 'PCM (float)', lossless: true, bits: null }];
	const levels: SignalInput['level'][] = ['original', 'high', 'low', null];
	for (const stream of streams) {
		for (const level of levels) {
			for (const deviceRate of [96000, 44100, null]) {
				const v = signalPath({ ...base, stream, level, deviceRate, gain: 0.5 });
				for (const r of v.rows) expect(r.value, r.label).toMatch(/^[\w ().,:;'·→/%-]{1,120}$/);
				expect(v.headline).toMatch(/^[\w ,./-]{1,40}$/);
			}
		}
	}
});
