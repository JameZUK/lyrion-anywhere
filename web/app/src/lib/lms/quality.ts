// The quality badge on Now playing ("FLAC 24/96 · Hi-Res", "MP3 320"), from the format
// details LMS reports for the current track (status tags o, T, I and r). Plain text only.
//
// Hi-Res means more than 16-bit or more than 48 kHz, on lossless sources only. The Android
// app has the same rules in Rust (mobile/src/model.rs, `quality_of`).

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

const CODECS: Record<string, [name: string, lossless: boolean]> = {
	flc: ['FLAC', true],
	flac: ['FLAC', true],
	alc: ['ALAC', true],
	alac: ['ALAC', true],
	wav: ['WAV', true],
	aif: ['AIFF', true],
	aiff: ['AIFF', true],
	pcm: ['PCM', true],
	ape: ['APE', true],
	wvp: ['WavPack', true],
	wmal: ['WMA Lossless', true],
	dsf: ['DSD', true],
	dff: ['DSD', true],
	mp3: ['MP3', false],
	aac: ['AAC', false],
	mp4: ['AAC', false],
	m4a: ['AAC', false],
	ogg: ['Ogg Vorbis', false],
	ops: ['Opus', false],
	opus: ['Opus', false],
	wma: ['WMA', false],
	wmap: ['WMA', false],
	mpc: ['Musepack', false],
};

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

/** 44100 → "44.1", 96000 → "96". */
export const khz = (hz: number): string =>
	hz % 1000 === 0 ? String(hz / 1000) : (hz / 1000).toLocaleString('en-GB', { maximumFractionDigits: 1, useGrouping: false });

/** The badge for a track's format details, or null when LMS gave no usable type. */
export function quality(t: { type?: unknown; samplerate?: unknown; samplesize?: unknown; bitrate?: unknown } | null | undefined): Quality | null {
	const kind = typeof t?.type === 'string' ? t.type.trim().toLowerCase() : '';
	if (!kind || kind.length > 12) return null;
	const known = CODECS[kind];
	const codec = known ? known[0] : /^[a-z0-9]{1,6}$/.test(kind) ? kind.toUpperCase() : null;
	if (!codec) return null;
	const lossless = known ? known[1] : false;
	const rate = int(t?.samplerate, 1000, 50_000_000);
	const bits = int(t?.samplesize, 1, 64);
	const kbps = kbpsOf(t?.bitrate);
	let label = codec;
	if (codec === 'DSD' && rate) label = `DSD${Math.round(rate / 44100)}`;
	else if (lossless && bits && rate) label = `${codec} ${bits}/${khz(rate)}`;
	else if (lossless && rate) label = `${codec} ${khz(rate)} kHz`;
	else if (!lossless && kbps) label = `${codec} ${kbps}`;
	const hiRes = lossless && ((bits !== null && bits > 16) || (rate !== null && rate > 48_000));
	return { codec, label, lossless, hiRes, rate, bits, kbps };
}

/** The badge's text: "FLAC 24/96 · Hi-Res". */
export const badgeText = (q: Quality): string => (q.hiRes ? `${q.label} · Hi-Res` : q.label);
