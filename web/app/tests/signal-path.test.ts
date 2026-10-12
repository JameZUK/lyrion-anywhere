// The web signal path (docs/plans: Wave 1 §5): what it says for this browser's own player,
// honestly, and how little it claims for other players.
import { quality } from '$lib/lms/quality';
import { signalPath, type SignalInput } from '$lib/player/signal-path';

const base: SignalInput = {
	quality: quality({ type: 'flc', samplesize: '24', samplerate: '96000' }),
	thisDevice: true,
	playerName: 'This browser',
	mode: { mode: 'worklet', source: null, reason: '' },
	volumeFollows: true,
	gain: 1,
	engineRate: 96000,
	deviceRate: 96000,
};
const value = (v: ReturnType<typeof signalPath>, label: string) => v.rows.find((r) => r.label === label)?.value;

test('lossless at the device rate and full volume: lossless to the browser', () => {
	const v = signalPath(base);
	expect(v.verdict).toBe('lossless');
	expect(value(v, 'Source')).toBe('FLAC 24/96 · Hi-Res · lossless');
	expect(value(v, 'Output')).toBe('Web Audio (AudioWorklet)');
	expect(value(v, 'Rate')).toMatch(/^96 kHz, the output device's own rate/);
	// It never claims more than the browser: the system may still change the sound.
	expect(v.notes.join(' ')).toMatch(/operating system/);
});

test('resampled, volume applied, media element and lossy sources are said plainly', () => {
	expect(signalPath({ ...base, deviceRate: 48000 }).verdict).toBe('resampled');
	expect(value(signalPath({ ...base, deviceRate: 48000 }), 'Rate')).toBe('96 kHz → 48 kHz, resampled by the browser');
	const quiet = signalPath({ ...base, gain: 0.5 });
	expect(quiet.verdict).toBe('volume');
	expect(value(quiet, 'Volume and DSP')).toMatch(/50% gain/);
	expect(value(signalPath({ ...base, volumeFollows: false }), 'Volume and DSP')).toMatch(/buttons/);
	expect(value(signalPath({ ...base, mode: { mode: 'media', source: 'managed', mime: 'audio/mp4; codecs="alac"', reason: '' } }), 'Output')).toBe('Media element (Apple Lossless in MP4)');
	expect(signalPath({ ...base, quality: quality({ type: 'mp3', bitrate: '320kbps' }) }).verdict).toBe('lossy');
	expect(signalPath({ ...base, quality: null }).verdict).toBe('unknown');
	// Unknown device rate: the rate alone, no resampling claim either way.
	expect(value(signalPath({ ...base, deviceRate: null }), 'Rate')).toBe('96 kHz');
});

test('another player: only the source is known', () => {
	const v = signalPath({ ...base, thisDevice: false, playerName: 'Kitchen' });
	expect(value(v, 'Decoding')).toBe('On Kitchen');
	expect(v.rows.some((r) => r.label === 'Rate')).toBe(false);
	expect(v.verdict).toBe('source');
	expect(signalPath({ ...base, thisDevice: false, quality: null }).verdict).toBe('unknown');
});

test('junk gains never show nonsense', () => {
	for (const gain of [NaN, Infinity, -1, 7]) {
		expect(value(signalPath({ ...base, gain }), 'Volume and DSP')).not.toMatch(/NaN|Infinity|-\d|\d{4,}%/);
	}
});
