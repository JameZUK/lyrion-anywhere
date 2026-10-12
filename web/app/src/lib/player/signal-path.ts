// The web app's signal path (opened from the quality badge): where the audio comes from,
// what decodes it, what changes it (volume), how the browser plays it and at what rate. It
// claims only what the page knows: after the browser's output, the operating system may
// mix or resample further, and the panel says so.
import { khz, type Quality } from '$lib/lms/quality';
import { MODE_LABELS, type ModeChoice } from './mode';

export interface SignalInput {
	/** The current track's format (from LMS). */
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
}

export type Verdict = 'lossless' | 'volume' | 'resampled' | 'lossy' | 'unknown';

export interface SignalPathView {
	rows: { label: string; value: string }[];
	verdict: Verdict;
	notes: string[];
}

export const VERDICT_LABELS: Record<Verdict, string> = {
	lossless: 'Lossless to the browser',
	volume: 'Lossless, volume applied',
	resampled: 'Resampled',
	lossy: 'Lossy source',
	unknown: 'Not known',
};

const rate = (hz: number) => `${khz(hz)} kHz`;

/** The signal path as rows of plain text, with an honest verdict. */
export function signalPath(i: SignalInput): SignalPathView {
	const q = i.quality;
	const rows: { label: string; value: string }[] = [];
	const notes: string[] = [];
	const source = q ? [q.label, q.hiRes ? 'Hi-Res' : '', q.lossless ? 'lossless' : 'lossy'].filter(Boolean).join(' · ') : 'Not reported';
	rows.push({ label: 'Source', value: source });
	if (!i.thisDevice) {
		rows.push({ label: 'Decoding', value: `On ${i.playerName || 'the player'}` });
		rows.push({ label: 'Volume and output', value: 'Set by that player' });
		notes.push('Details beyond the source are known only for this device.');
		return { rows, verdict: !q ? 'unknown' : q.lossless ? 'unknown' : 'lossy', notes };
	}
	rows.push({ label: 'Decoding', value: 'In this browser, by the player engine (WebAssembly)' });
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
		const how = i.mode.mode === 'media' ? `${MODE_LABELS.media} (${/alac/i.test(i.mode.mime ?? '') ? 'Apple Lossless' : 'FLAC'} in MP4)` : `${MODE_LABELS.worklet} (AudioWorklet)`;
		rows.push({ label: 'Output', value: how });
	} else {
		rows.push({ label: 'Output', value: 'Not playing here' });
	}
	const resampled = !!i.engineRate && !!i.deviceRate && i.engineRate !== i.deviceRate;
	if (i.engineRate) {
		rows.push({
			label: 'Rate',
			value: resampled ? `${rate(i.engineRate)} → ${rate(i.deviceRate!)}, resampled by the browser` : i.deviceRate ? `${rate(i.engineRate)}, the output device's own rate` : rate(i.engineRate),
		});
	}
	notes.push('After the browser, the operating system may still mix or resample the sound.');
	const verdict: Verdict = !q ? 'unknown' : !q.lossless ? 'lossy' : resampled ? 'resampled' : scaled ? 'volume' : 'lossless';
	return { rows, verdict, notes };
}
