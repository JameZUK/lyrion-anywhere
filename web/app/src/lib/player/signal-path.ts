// The web app's signal path (opened from the quality badge): the track as it is in the
// library, the stream that actually reaches this device (which differs when the gateway
// re-encodes to Opus for a lower-data level), what decodes it, what changes it (volume), how
// the browser plays it and at what rate. It claims only what the page knows: after the
// browser's output, the operating system may mix or resample further, and the panel says so.
import { khz, type Quality } from '#lib/lms/quality';
import { MODE_LABELS, type ModeChoice } from './mode';
import { LEVEL_KBPS, LEVEL_LABELS, reencoded, type StreamQuality, type StreamSource } from './stream-quality';

export interface SignalInput {
	/** The current track's format (from LMS): the track as stored in the library. */
	quality: Quality | null;
	/** The selected player is this browser. */
	thisDevice: boolean;
	/** The selected player's name (for other players). */
	playerName: string;
	/** How this browser plays (null when it isn't playing here). */
	mode: ModeChoice | null;
	/** Volume follows LMS (digital gain here); false: held at full scale for the device's buttons. */
	volumeFollows: boolean | null;
	/** LMS's volume as a linear gain (the louder side), 0–1. */
	gain: number;
	/** The rate the player engine outputs at (Hz), when known. */
	engineRate: number | null;
	/** The output device's own rate (Hz), when known. */
	deviceRate: number | null;
	/** What the player engine is decoding: the stream as it arrives (null until known). */
	stream: StreamSource | null;
	/** The streaming quality this device asked the gateway for (null for other players). */
	level: StreamQuality | null;
}

export type Verdict = 'lossless' | 'volume' | 'resampled' | 'stream' | 'lossy' | 'source' | 'unknown';

export interface SignalPathView {
	rows: { label: string; value: string }[];
	verdict: Verdict;
	/** The verdict as shown: `VERDICT_LABELS[verdict]`, with the bit rate for a re-encoded stream. */
	headline: string;
	notes: string[];
}

export const VERDICT_LABELS: Record<Verdict, string> = {
	lossless: 'Lossless to the browser',
	volume: 'Lossless, volume applied',
	resampled: 'Resampled',
	stream: 'Re-encoded to Opus',
	lossy: 'Lossy source',
	source: 'Lossless source',
	unknown: 'Not known',
};

const rate = (hz: number) => `${khz(hz)} kHz`;

/** "Opus 160 kb/s" for a level, or just "Opus" when the level isn't known. */
function opusLabel(level: StreamQuality | null): string {
	const kbps = level ? LEVEL_KBPS[level] : null;
	return kbps ? `Opus ${kbps} kb/s` : 'Opus';
}

/** The signal path as rows of plain text, with an honest verdict. */
export function signalPath(i: SignalInput): SignalPathView {
	const q = i.quality;
	const rows: { label: string; value: string }[] = [];
	const notes: string[] = [];
	const source = q ? [q.label, q.hiRes ? 'Hi-Res' : '', q.lossless ? 'lossless' : 'lossy'].filter(Boolean).join(' · ') : 'Not reported';
	rows.push({ label: 'Track in the library', value: source });
	if (!i.thisDevice) {
		rows.push({ label: 'Decoding', value: `On ${i.playerName || 'the player'}` });
		rows.push({ label: 'Volume and output', value: 'Set by that player' });
		notes.push('Details beyond the track are known only for this device.');
		return { rows, verdict: !q ? 'unknown' : q.lossless ? 'source' : 'lossy', headline: VERDICT_LABELS[!q ? 'unknown' : q.lossless ? 'source' : 'lossy'], notes };
	}

	// The stream as it actually arrives here. The gateway re-encodes only FLAC, only to Opus,
	// and only for the lower-data levels; anything else is the track as stored.
	const s = i.stream;
	const opus = reencoded(q?.codec ?? null, s, i.level);
	let stream: string;
	if (!s) stream = 'Not known yet';
	else if (opus) stream = `${opusLabel(i.level)}, re-encoded by the gateway${i.level && i.level !== 'original' ? ` for the ${LEVEL_LABELS[i.level]} setting` : ''}`;
	else stream = `${s.codec}${s.lossless && s.bits ? ` ${s.bits}-bit` : ''}, as stored (not re-encoded)`;
	rows.push({ label: 'Stream to this device', value: stream });

	rows.push({ label: 'Decoding', value: 'On this device, by the player engine (WebAssembly)' });
	const gain = Number.isFinite(i.gain) ? Math.min(1, Math.max(0, i.gain)) : 1;
	const scaled = i.volumeFollows === true && gain < 0.999;
	rows.push({
		label: 'Volume and DSP',
		value:
			i.volumeFollows === false
				? "Full scale: this device's buttons set the level"
				: scaled
					? `LMS volume applied digitally (${Math.round(gain * 100)}% gain); no other processing`
					: 'Untouched (volume at 100%); no other processing',
	});
	if (i.mode) {
		const how =
			i.mode.mode === 'media'
				? `${MODE_LABELS.media} (the decoded audio, re-wrapped losslessly as ${/alac/i.test(i.mode.mime ?? '') ? 'Apple Lossless' : 'FLAC'} in MP4)`
				: `${MODE_LABELS.worklet} (AudioWorklet)`;
		rows.push({ label: 'Output', value: how });
	} else {
		rows.push({ label: 'Output', value: 'Not playing here' });
	}
	const resampled = !!i.engineRate && !!i.deviceRate && i.engineRate !== i.deviceRate;
	if (i.engineRate) {
		let value = resampled ? `${rate(i.engineRate)} → ${rate(i.deviceRate!)}, resampled by the browser` : i.deviceRate ? `${rate(i.engineRate)}, the output device's own rate` : rate(i.engineRate);
		// 96 kHz in the library but 48 kHz here is the Opus stream, not something the browser did.
		if (s?.codec === 'Opus' && i.engineRate === 48_000) value += ' (Opus is always 48 kHz)';
		rows.push({ label: 'Rate', value });
	}
	if (opus) notes.push('For the track as stored, set Streaming quality to Original in Settings › Playback.');
	notes.push('After the browser, the operating system may still mix or resample the sound.');

	const lossy = (!!q && !q.lossless) || (!!s && !s.lossless);
	// A lower level was asked for but nothing is decoded yet: whether the gateway re-encoded
	// this stream isn't known, so nothing is claimed.
	const pending = !s && i.level !== null && i.level !== 'original';
	const verdict: Verdict = (!q && !s) || pending ? 'unknown' : opus ? 'stream' : lossy ? 'lossy' : resampled ? 'resampled' : scaled ? 'volume' : 'lossless';
	const headline = verdict === 'stream' ? `Re-encoded to ${opusLabel(i.level)}` : pending ? 'Not known yet' : VERDICT_LABELS[verdict];
	return { rows, verdict, headline, notes };
}
