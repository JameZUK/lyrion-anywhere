// The quality badge and the signal-path block diagram (docs/plans/signal-path-redesign.md):
// a TypeScript port of the shared core's `anywhere_core::quality::describe`, held to the same
// vectors (core/tests/fixtures/quality-vectors.json) byte for byte, so the web and Android
// cannot drift. Field names and JSON shapes are the Rust's (`snake_case`).
//
// The verdict is decided by codecs and re-encoding only: a lossy file is Lossy; a lossless
// file heard through a lossy stream is Transcoded; a lossless stream arriving as stored is
// Hi-Res (more than 16 bits or more than 48 kHz) or Lossless. Device resampling, DSP,
// ReplayGain and volume never change it.
//
// Everything here is total: any input gives a diagram whose strings are short and free of
// control characters. Numbers out of range (or not whole) are treated as unknown.

/** The longest text taken from an input string (names, labels), in characters. */
const MAX_NAME = 40;

// ---------------------------------------------------------------------------------------
// Inputs.
// ---------------------------------------------------------------------------------------

export type Player = { kind: 'local' } | { kind: 'remote'; name: string } | { kind: 'offline'; format: string };

export interface FileFormat {
	/** LMS's content type, e.g. "flc", "mp3", "ops" (any case). */
	kind?: string | null;
	rate?: number | null;
	bits?: number | null;
	kbps?: number | null;
	channels?: number | null;
}

export interface Served {
	/** "flac", "pcm", "mp3", "aac", "ogg", "alac" or "opus". */
	codec?: string | null;
	rate?: number | null;
	bits?: number | null;
	channels?: number | null;
	/** Server ReplayGain applied to the stream, dB (absent: none). */
	gain_db?: number | null;
}

export type OpusLevel = 'high' | 'low';
export type Sent = { mode: 'passthrough' } | { mode: 'opus'; kbps?: number | null; level?: OpusLevel | null };

export interface Decoded {
	/** The engine's display name: "FLAC", "Opus", "MP3", "PCM", … */
	codec?: string | null;
	rate?: number | null;
	bits?: number | null;
	channels?: number | null;
	lossless?: boolean;
}

export interface Device {
	/** "Speakers", "USB DAC", … */
	name?: string | null;
	/** The output's rate, Hz. */
	rate?: number | null;
	/** Every sample reaches the device unchanged. */
	bit_perfect?: boolean;
	/** The engine itself leaves every sample unchanged (the platform may not). */
	engine_bit_perfect?: boolean;
	/** The rate is converted on the way. */
	resampled?: boolean;
	/** DSP the listener turned on changes the samples. */
	dsp_active?: boolean;
}

export type Level = 'original' | 'high' | 'low';
export type Network = 'wifi' | 'mobile';
export type Origin =
	| { kind: 'setting'; network?: Network | null; level: Level }
	| { kind: 'override'; level: Level }
	| { kind: 'automatic'; level: Level };

export interface QualityInputs {
	player?: Player;
	file?: FileFormat | null;
	served?: Served | null;
	sent?: Sent | null;
	decoded?: Decoded | null;
	device?: Device | null;
	origin?: Origin | null;
	seekable?: boolean;
}

// ---------------------------------------------------------------------------------------
// Output.
// ---------------------------------------------------------------------------------------

export type Verdict = 'hi_res' | 'lossless' | 'transcoded' | 'lossy';
export type BlockKind = 'file' | 'lms' | 'gateway' | 'device' | 'download' | 'player';

export interface Badge {
	verdict: Verdict;
	/** What is heard: "FLAC 24/96", "Opus 160", "MP3 320". */
	stream: string;
	/** The full text: "Transcoded · Opus 160". */
	text: string;
}

export interface Block {
	kind: BlockKind;
	title: string;
	/** The format at this point; empty when unknown. */
	format: string;
	/** One line of what happened here; may be empty. */
	note: string;
	/** This block lowers the verdict (drawn in the amber tint). */
	lowers: boolean;
}

export interface Switch {
	levels: [Level, Level, Level];
	in_force: Level;
	origin_text: string;
	overridden: boolean;
	seekable: boolean;
}

export interface Diagram {
	badge: Badge | null;
	blocks: Block[];
	switch: Switch | null;
}

const WORDS: Record<Verdict, string> = { hi_res: 'Hi-Res', lossless: 'Lossless', transcoded: 'Transcoded', lossy: 'Lossy' };
const LEVEL_NAMES: Record<Level, string> = { original: 'Original', high: 'High', low: 'Data saver' };

export const verdictWord = (v: Verdict): string => WORDS[v];
/** Transcoded and Lossy are drawn in the amber tint. */
export const isLowered = (v: Verdict): boolean => v === 'transcoded' || v === 'lossy';
/** The level's name as the switch and the reasons show it. */
export const levelName = (l: Level): string => LEVEL_NAMES[l] ?? 'Original';

// ---------------------------------------------------------------------------------------
// Text helpers with Rust's semantics.
// ---------------------------------------------------------------------------------------

// Rust's `char::is_whitespace` (the Unicode White_Space property), for `str::trim`.
const WS = '\\t\\n\\v\\f\\r \\u0085\\u00a0\\u1680\\u2000-\\u200a\\u2028\\u2029\\u202f\\u205f\\u3000';
const TRIM = new RegExp(`^[${WS}]+|[${WS}]+$`, 'g');
const TRIM_END = new RegExp(`[${WS}]+$`);
const trim = (s: string) => s.replace(TRIM, '');
const asciiLower = (s: string) => s.replace(/[A-Z]/g, (c) => c.toLowerCase());
const asciiUpper = (s: string) => s.replace(/[a-z]/g, (c) => c.toUpperCase());
const UTF8 = new TextEncoder();
const utf8Len = (s: string) => UTF8.encode(s).length;

// The characters `clean` drops, as the core's `is_invisible`: controls (`Cc`), the line and
// paragraph separators, every format character (`Cf`, Unicode 16) and the other characters
// that draw nothing (the combining grapheme joiner, the Hangul fillers, the variation
// selectors), any of which could make a name read as something else.
const INVISIBLE_CLASS =
	'[\\u0000-\\u001f\\u007f-\\u009f\\u00ad\\u034f\\u0600-\\u0605\\u061c\\u06dd\\u070f\\u0890-\\u0891\\u08e2' +
	'\\u115f-\\u1160\\u180e\\u200b-\\u200f\\u2028-\\u202e\\u2060-\\u206f\\u3164' +
	'\\ufe00-\\ufe0f\\ufeff\\uffa0\\ufff9-\\ufffb' +
	'\\u{110bd}\\u{110cd}\\u{13430}-\\u{1343f}\\u{1bca0}-\\u{1bca3}\\u{1d173}-\\u{1d17a}' +
	'\\u{e0000}-\\u{e007f}\\u{e0100}-\\u{e01ef}]';
const INVISIBLE = new RegExp(INVISIBLE_CLASS, 'gu');
const ONE_INVISIBLE = new RegExp(`^${INVISIBLE_CLASS}$`, 'u');

/** Whether `clean` drops this character (one code point), as the core's `is_invisible`. */
export const isInvisible = (c: string): boolean => ONE_INVISIBLE.test(c);

/** Text from an input: the invisible characters dropped, trimmed, at most `max` characters. */
export function clean(s: string, max: number): string {
	const t = trim(s.replace(INVISIBLE, ''));
	return Array.from(t).slice(0, max).join('').replace(TRIM_END, '');
}

function capitalise(s: string): string {
	const [first, ...rest] = Array.from(s);
	if (first !== undefined && /\p{Lowercase}/u.test(first)) return first.toUpperCase() + rest.join('');
	return s;
}

/** "−3.2 dB" (typographic minus), "+1.5 dB"; ties at one decimal go to even, as Rust's do. */
export function fmtDb(db: number): string {
	const neg = db < 0;
	const a = Math.abs(db);
	let s: string;
	const q = a * 4; // exact: a tie at one decimal place is a multiple of 0.25 that is odd
	if (Number.isInteger(q) && q % 2 === 1) {
		const tenths = Math.floor(a * 10); // exact for these values
		const n = tenths % 2 === 0 ? tenths : tenths + 1;
		s = `${Math.floor(n / 10)}.${n % 10}`;
	} else s = a.toFixed(1);
	return neg ? `−${s} dB` : `+${s} dB`;
}

// ---------------------------------------------------------------------------------------
// The codec table and labels.
// ---------------------------------------------------------------------------------------

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

/** LMS's content type or a stream token to the codec's display name and whether it is lossless. */
export function codec(kind: string): [string, boolean] | null {
	const k = trim(kind);
	if (utf8Len(k) > 12) return null;
	const c = Object.hasOwn(CODECS, asciiLower(k)) ? CODECS[asciiLower(k)] : undefined;
	return c ? [c[0], c[1]] : null;
}

/** `codec`, with LMS's fallback: a short alphanumeric type shown upper-cased, taken as lossy. */
export function codecName(kind: string): [string, boolean] | null {
	const c = codec(kind);
	if (c) return c;
	const k = trim(kind);
	return /^[A-Za-z0-9]{1,6}$/.test(k) ? [asciiUpper(k), false] : null;
}

/** 44100 → "44.1", 48000 → "48", 22050 → "22.1", 11025 → "11". */
export function khz(hz: number): string {
	if (hz % 1000 === 0) return String(hz / 1000);
	const tenths = Math.floor((hz + 50) / 100);
	return tenths % 10 === 0 ? String(tenths / 10) : `${Math.floor(tenths / 10)}.${tenths % 10}`;
}

/** "FLAC 24/96", "FLAC 44.1 kHz", "MP3 320", "DSD64", or the codec's name. */
export function formatLabel(name: string, lossless: boolean, bits: number | null, rate: number | null, kbps: number | null): string {
	if (name === 'DSD') {
		const n = rate !== null ? Math.round(rate / 44_100) : null;
		return n !== null && n >= 32 ? `DSD${n}` : 'DSD';
	}
	if (lossless && bits !== null && rate !== null) return `${name} ${bits}/${khz(rate)}`;
	if (lossless && bits === null && rate !== null) return `${name} ${khz(rate)} kHz`;
	if (!lossless && kbps !== null) return `${name} ${kbps}`;
	return name;
}

/** Hi-Res: lossless, and more than 16 bits or more than 48 kHz. */
export const isHiRes = (lossless: boolean, bits: number | null, rate: number | null): boolean =>
	lossless && ((bits !== null && bits > 16) || (rate !== null && rate > 48_000));

/** "High, on mobile data", "Data saver, chosen in Settings", "Original, overridden here". */
export function originText(o: Origin): string {
	let why: string;
	if (o.kind === 'setting') why = o.network === 'wifi' ? 'on Wi-Fi' : o.network === 'mobile' ? 'on mobile data' : 'chosen in Settings';
	else if (o.kind === 'override') why = 'overridden here';
	else why = 'chosen automatically';
	return `${levelName(o.level)}, ${why}`;
}

// ---------------------------------------------------------------------------------------
// The rules.
// ---------------------------------------------------------------------------------------

interface Fmt {
	name: string;
	lossless: boolean;
	bits: number | null;
	rate: number | null;
	kbps: number | null;
	channels: number | null;
}

const label = (f: Fmt) => formatLabel(f.name, f.lossless, f.bits, f.rate, f.kbps);
const labelWithChannels = (f: Fmt) => withChannels(label(f), f.channels);
const hiRes = (f: Fmt) => isHiRes(f.lossless, f.bits, f.rate);
const fmtVerdict = (f: Fmt): Verdict => (!f.lossless ? 'lossy' : hiRes(f) ? 'hi_res' : 'lossless');

function canon(n: string): string {
	if (n === 'WAV' || n === 'AIFF' || n === 'PCM (float)') return 'PCM';
	if (n === 'Vorbis') return 'Ogg Vorbis';
	return n;
}
/** The same codec under any of its names (WAV and AIFF reach a player as PCM, …). */
const sameCodec = (a: Fmt, b: Fmt) => canon(a.name) === canon(b.name);

function withChannels(l: string, channels: number | null): string {
	if (channels === 1) return `${l}, mono`;
	if (channels !== null && channels > 2) return `${l}, ${channels} channels`;
	return l;
}

function inRange(v: unknown, lo: number, hi: number): number | null {
	return typeof v === 'number' && Number.isInteger(v) && v >= lo && v <= hi ? v : null;
}
const rateOf = (v: unknown) => inRange(v, 1000, 50_000_000);
const bitsOf = (v: unknown) => inRange(v, 1, 64);
const kbpsOf = (v: unknown) => inRange(v, 8, 100_000);
const channelsOf = (v: unknown) => inRange(v, 1, 64);
const str = (v: unknown): string | null => (typeof v === 'string' ? v : null);

function fileFmt(f: FileFormat): Fmt | null {
	const kind = str(f.kind);
	if (kind === null) return null;
	const c = codecName(kind);
	if (!c) return null;
	return { name: c[0], lossless: c[1], bits: bitsOf(f.bits), rate: rateOf(f.rate), kbps: kbpsOf(f.kbps), channels: channelsOf(f.channels) };
}

const isOgg = (name: string) => name === 'Ogg Vorbis' || name === 'Opus';

/**
 * LMS's stream; a lossy stream in the file's codec borrows the file's bit rate. A lossless
 * stream whose message did not carry its depth or rate takes them from the decoder when the
 * gateway passed it through in the same codec, and otherwise from the file when it is in the
 * file's codec.
 */
function servedFmt(s: Served, file: Fmt | null, sent: Sent | null, decoded: Fmt | null): Fmt | null {
	const token = str(s.codec);
	if (token === null) return null;
	const ogg = asciiLower(trim(token)) === 'ogg';
	let name: string;
	let lossless: boolean;
	if (ogg && file && isOgg(file.name)) [name, lossless] = [file.name, false];
	else if (ogg) [name, lossless] = ['Ogg', false];
	else {
		const c = codec(token);
		if (!c) return null;
		[name, lossless] = c;
	}
	const f: Fmt = { name, lossless, bits: bitsOf(s.bits), rate: rateOf(s.rate), kbps: null, channels: channelsOf(s.channels) };
	if (lossless && sent?.mode === 'passthrough' && decoded && decoded.lossless && sameCodec(decoded, f)) {
		f.bits = f.bits ?? decoded.bits;
		f.rate = f.rate ?? decoded.rate;
	}
	if (file && sameCodec(file, f)) {
		if (lossless) {
			f.bits = f.bits ?? file.bits;
			f.rate = f.rate ?? file.rate;
		} else f.kbps = file.kbps;
	}
	return f;
}

function decodedFmt(d: Decoded): Fmt | null {
	const c = str(d.codec);
	if (c === null) return null;
	const name = clean(c, 20);
	if (!name) return null;
	const lossless = d.lossless === true;
	return { name, lossless, bits: lossless ? bitsOf(d.bits) : null, rate: rateOf(d.rate), kbps: null, channels: channelsOf(d.channels) };
}

const opusFmt = (kbps: unknown): Fmt => ({ name: 'Opus', lossless: false, bits: null, rate: null, kbps: kbpsOf(kbps), channels: null });

/** The decoder's format as the device block shows it: "FLAC 24/96", "Opus 48 kHz". */
function decodedLabel(d: Fmt): string {
	const l = !d.lossless && d.rate !== null ? `${d.name} ${khz(d.rate)} kHz` : formatLabel(d.name, d.lossless, d.bits, d.rate, null);
	return withChannels(l, d.channels);
}

function isOpusSent(s: Sent | null | undefined): s is { mode: 'opus'; kbps?: number | null; level?: OpusLevel | null } {
	return !!s && s.mode === 'opus';
}

function heardLocal(file: Fmt | null, served: Fmt | null, sent: Sent | null, decoded: Fmt | null): Fmt | null {
	const opus = isOpusSent(sent) ? sent : null;
	if (decoded) {
		const h = { ...decoded };
		if (!h.lossless) {
			if (h.name === 'Opus' && opus) h.kbps = kbpsOf(opus.kbps);
			else {
				const src = (served && !served.lossless && sameCodec(served, h) ? served : null) ?? (file && !file.lossless && sameCodec(file, h) ? file : null);
				if (src) h.kbps = src.kbps;
			}
		}
		return h;
	}
	if (opus) return opusFmt(opus.kbps);
	const h = served ?? file;
	return h ? { ...h } : null;
}

function badge(verdict: Verdict, stream: string): Badge {
	return { verdict, stream, text: `${WORDS[verdict]} · ${stream}` };
}

/**
 * A lossy source is Lossy; a lossless source heard through a lossy stream is Transcoded;
 * otherwise the heard stream's own verdict. The source is the file when LMS reports its type,
 * else LMS's stream (`servedLossless`).
 */
function verdictOf(file: Fmt | null, servedLossless: boolean | null, heard: Fmt): Verdict {
	const source = file ? file.lossless : servedLossless;
	if (source === false) return 'lossy';
	if (source === true && !heard.lossless) return 'transcoded';
	return fmtVerdict(heard);
}

const block = (kind: BlockKind, title: string, format: string, note: string): Block => ({ kind, title, format, note, lowers: false });

function fileBlock(file: Fmt | null): Block {
	if (!file) return block('file', 'File', '', 'Format not reported');
	const v = fmtVerdict(file);
	return block('file', 'File', labelWithChannels(file), v === 'hi_res' ? 'Hi-Res lossless' : v === 'lossless' ? 'Lossless' : 'Lossy');
}

function deviceNote(d: Device | null): string {
	if (!d) return '';
	const parts: string[] = [];
	if (d.bit_perfect === true) parts.push('bit-perfect');
	else {
		if (d.resampled === true) {
			const r = rateOf(d.rate);
			parts.push(r !== null ? `resampled to ${khz(r)} kHz` : 'resampled');
		}
		if (d.dsp_active === true) parts.push('processed by DSP you turned on');
	}
	const clauses = parts.join(', ');
	// The device's own name is shown as the platform gives it; only a note that starts with
	// one of our clauses is capitalised.
	const n = str(d.name);
	const name = n !== null ? clean(n, MAX_NAME) : '';
	if (name) return clauses ? `${name}, ${clauses}` : name;
	return capitalise(clauses);
}

const deviceBlock = (decoded: Fmt | null, device: Device | null): Block =>
	block('device', 'This device', decoded ? decodedLabel(decoded) : '', deviceNote(device));

function lmsChanges(file: Fmt | null, served: Fmt | null, gain: unknown): string | null {
	const parts: string[] = [];
	if (file && served) {
		if (!sameCodec(file, served)) parts.push(`converted from ${file.name}`);
		if (file.name !== 'DSD' && served.lossless) {
			const rate = file.rate !== null && served.rate !== null && served.rate < file.rate;
			const bits = file.bits !== null && served.bits !== null && served.bits < file.bits && file.lossless;
			let what: string | null = null;
			if (bits && rate && served.bits !== null && served.rate !== null) what = `${served.bits}-bit, ${khz(served.rate)} kHz`;
			else if (bits && !rate && served.bits !== null) what = `${served.bits}-bit`;
			else if (!bits && rate && served.rate !== null) what = `${khz(served.rate)} kHz`;
			if (what !== null) parts.push(`lowered to ${what}`);
		}
	}
	if (typeof gain === 'number' && Number.isFinite(gain) && Math.abs(gain) >= 0.05 && Math.abs(gain) <= 100) parts.push(`ReplayGain ${fmtDb(gain)}`);
	if (!parts.length) return null;
	return capitalise(parts.join(', '));
}

/**
 * The level the stream heard was sent at, when the stream message shows it: an Opus
 * re-encode is the level it was made for (High when the message does not say); a FLAC stream
 * passed through is Original. The gateway re-encodes only FLAC, so any other stream passed
 * through shows none.
 */
function sentLevel(sent: Sent, served: Served | null): Level | null {
	if (isOpusSent(sent)) return sent.level === 'low' ? 'low' : 'high';
	if (sent.mode !== 'passthrough') return null;
	const token = served ? str(served.codec) : null;
	const c = token !== null ? codec(token) : null;
	return c && c[0] === 'FLAC' ? 'original' : null;
}

/** The reason for the level: the origin's when in force, or the chosen level still pending. */
interface Reason {
	pending: boolean;
	text: string;
}

/** The origin's reason when its level is the one heard (or nothing says), else "{Level} applies from the next track". */
function reasonOf(origin: Origin, heard: Level | null): Reason {
	if (heard !== null && heard !== origin.level) return { pending: true, text: `${levelName(origin.level)} applies from the next track` };
	return { pending: false, text: originText(origin) };
}

const heardLevel = (i: QualityInputs): Level | null => (i.sent ? sentLevel(i.sent, i.served ?? null) : null);

/**
 * What the gateway block says it did. With nothing from the gateway but a lossy stream decoded
 * from a lossless source (`reencodeUnreported`), its stream message is still on its way.
 */
function gatewayBlock(i: QualityInputs, out: Fmt | null, reencodeUnreported: boolean): Block {
	const sent = i.sent ?? null;
	const reason = i.origin ? reasonOf(i.origin, heardLevel(i)) : null;
	let format = '';
	let note = '';
	if (sent && sent.mode === 'passthrough') {
		format = out ? labelWithChannels(out) : '';
		note = reason?.pending ? `Passed through unchanged; ${reason.text}` : 'Passed through unchanged';
	} else if (isOpusSent(sent)) {
		const k = kbpsOf(sent.kbps);
		const action = k !== null ? `Re-encoded to Opus ${k} kb/s` : 'Re-encoded to Opus';
		note = !reason ? action : reason.pending ? `${action}; ${reason.text}` : `${action} (${reason.text})`;
		format = label(opusFmt(sent.kbps));
	} else if (!sent && reencodeUnreported) note = 'Re-encoded (details to follow)';
	return block('gateway', 'Lyrion gateway', format, note);
}

/** The level in force is the one the stream heard was sent at, when shown; else the origin's; else Original. */
function switchOf(i: QualityInputs): Switch {
	const origin = i.origin ?? null;
	const heard = heardLevel(i);
	return {
		levels: ['original', 'high', 'low'],
		in_force: heard ?? origin?.level ?? 'original',
		origin_text: origin ? reasonOf(origin, heard).text : '',
		overridden: origin?.kind === 'override',
		seekable: i.seekable === true,
	};
}

function setLowers(blocks: Block[], kind: BlockKind) {
	const b = blocks.find((x) => x.kind === kind);
	if (b) b.lowers = true;
}

/** The badge, the diagram and the switch for a playing track. */
export function describe(inputs: QualityInputs): Diagram {
	const file = inputs.file ? fileFmt(inputs.file) : null;
	const decoded = inputs.decoded ? decodedFmt(inputs.decoded) : null;
	const player: Player = inputs.player ?? { kind: 'local' };

	if (player.kind === 'remote') {
		const name = clean(str(player.name) ?? '', MAX_NAME);
		const fb = fileBlock(file);
		const blocks = [fb, block('player', name || 'Player', '', 'Plays the stream from LMS')];
		let b: Badge | null = null;
		if (file) {
			const v = fmtVerdict(file);
			if (v === 'lossy') fb.lowers = true;
			b = badge(v, label(file));
		}
		return { badge: b, blocks, switch: null };
	}

	if (player.kind === 'offline') {
		const l = clean(str(player.format) ?? '', MAX_NAME);
		const db = block('download', 'Download', l, 'Saved on this device');
		const blocks = [db, deviceBlock(decoded, inputs.device ?? null)];
		let b: Badge | null = null;
		if (decoded) {
			const v = verdictOf(file, null, decoded);
			if (isLowered(v)) db.lowers = true;
			b = badge(v, l || label(decoded));
		}
		return { badge: b, blocks, switch: null };
	}

	const sent = inputs.sent ?? null;
	const served = inputs.served ? servedFmt(inputs.served, file, sent, decoded) : null;
	const blocks = [fileBlock(file)];
	const lms = lmsChanges(file, served, inputs.served?.gain_db);
	const hasLms = lms !== null;
	if (lms !== null) blocks.push(block('lms', 'LMS', served ? labelWithChannels(served) : '', lms));
	const servedLossless = served ? served.lossless : null;
	const sourceLossless = file ? file.lossless : servedLossless;
	const servedLossy = servedLossless === false;
	// The gateway has said nothing yet, but a lossy stream is decoded from a lossless source
	// that LMS did not convert: only the gateway can have re-encoded it.
	const reencodeUnreported = !sent && !!decoded && !decoded.lossless && sourceLossless === true && !servedLossy;
	const reencoded = isOpusSent(sent) || reencodeUnreported;
	blocks.push(gatewayBlock(inputs, served ?? file, reencodeUnreported));
	blocks.push(deviceBlock(decoded, inputs.device ?? null));

	const heard = heardLocal(file, served, sent, decoded);
	let b: Badge | null = null;
	if (heard) {
		const v = verdictOf(file, servedLossless, heard);
		const fileLossy = !!file && !file.lossless;
		const down = v === 'transcoded' || v === 'lossy';
		let lowering: BlockKind | null = null;
		if (fileLossy) lowering = 'file';
		else if (down && servedLossy) lowering = 'lms';
		else if (down && reencoded) lowering = 'gateway';
		// LMS's conversion took a Hi-Res file to a stream that is not (lowered it, or
		// converted DSD to 16/44.1).
		else if (v === 'lossless' && hasLms && !!file && hiRes(file)) lowering = 'lms';
		if (lowering) setLowers(blocks, lowering);
		b = badge(v, label(heard));
	}
	return { badge: b, blocks, switch: switchOf(inputs) };
}
