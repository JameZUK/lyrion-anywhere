// The library file's format, from the details LMS reports for a track (status tags o, T, I
// and r): "FLAC 24/96", "MP3 320". The codec table, the labels and the Hi-Res rule are the
// shared core's (lib/player/quality-path.ts, a port of core::quality), so the file reads the
// same everywhere. The quality badge itself (what is heard: "Transcoded · Opus 160") is the
// diagram's (see quality-path's `describe`), never this alone.
import { codecName, formatLabel, isHiRes } from '#lib/player/quality-path';

export { khz } from '#lib/player/quality-path';

export interface Quality {
	/** The codec's name ("FLAC", "MP3", …). */
	codec: string;
	/** "FLAC 24/96", "FLAC 44.1 kHz", "MP3 320", "Opus". */
	label: string;
	lossless: boolean;
	hiRes: boolean;
	/** Sample rate (Hz) and size (bits), when LMS gave them. */
	rate: number | null;
	bits: number | null;
	/** Bit rate (kbps), when LMS gave one. */
	kbps: number | null;
}

function int(v: unknown, lo: number, hi: number): number | null {
	const n = typeof v === 'number' ? v : typeof v === 'string' && /^\s*\d{1,9}(\.\d+)?\s*$/.test(v) ? Number(v) : NaN;
	return Number.isFinite(n) && n >= lo && n <= hi ? Math.round(n) : null;
}

/** LMS's bit rate ("320kbps", "873kbps VBR", "320000") in kbps, or null. */
export function kbpsOf(v: unknown): number | null {
	if (typeof v === 'number') return v >= 8000 ? int(v / 1000, 8, 100_000) : int(v, 8, 100_000);
	if (typeof v !== 'string' || v.length > 40) return null;
	const m = v.match(/^\s*(\d{1,7})(?:\.\d+)?\s*(k?)bps\b/i) ?? v.match(/^\s*(\d{1,7})\s*$/);
	if (!m) return null;
	const n = Number(m[1]);
	return int(m[2] || n < 8000 ? n : n / 1000, 8, 100_000);
}

/** The file's format, or null when LMS gave no usable type. */
export function quality(t: { type?: unknown; samplerate?: unknown; samplesize?: unknown; bitrate?: unknown } | null | undefined): Quality | null {
	const c = typeof t?.type === 'string' ? codecName(t.type) : null;
	if (!c) return null;
	const [codec, lossless] = c;
	const rate = int(t?.samplerate, 1000, 50_000_000);
	const bits = int(t?.samplesize, 1, 64);
	const kbps = kbpsOf(t?.bitrate);
	return { codec, label: formatLabel(codec, lossless, bits, rate, kbps), lossless, hiRes: isHiRes(lossless, bits, rate), rate, bits, kbps };
}
