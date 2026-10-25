// Streaming quality for this browser's player (docs/plans/streaming-quality.md): the level
// asked of the gateway, chosen in Settings, automatically from the network, or overridden from
// the signal path for this session (docs/plans/signal-path-redesign.md, decision 5).
import type { Origin, Sent, Served } from './quality-path';

export type StreamQuality = 'original' | 'high' | 'low';
export type QualityPreference = 'auto' | StreamQuality;

export const PREFERENCES: QualityPreference[] = ['auto', 'original', 'high', 'low'];

/** What the Network Information API says, where the browser has it (not Safari). */
export interface ConnectionInfo {
	type?: string;
	saveData?: boolean;
}

/**
 * The level for "Automatic": Data saver when the browser is asked to save data, High on a
 * mobile connection, otherwise Original. Without the API (Safari, so iPhone and iPad), or
 * when it doesn't say, Original.
 */
export function autoQuality(c: ConnectionInfo | null | undefined): StreamQuality {
	if (!c) return 'original';
	if (c.saveData === true) return 'low';
	if (c.type === 'cellular') return 'high';
	return 'original';
}

export function effectiveQuality(pref: QualityPreference, c: ConnectionInfo | null | undefined): StreamQuality {
	return pref === 'auto' ? autoQuality(c) : pref;
}

/** What each level means, for Settings and the signal path. */
export const LEVEL_LABELS: Record<StreamQuality, string> = { original: 'Original', high: 'High', low: 'Data saver' };
/** The Opus bit rate (kb/s) a lower-data level asks the gateway for. */
export const LEVEL_KBPS: Record<StreamQuality, number | null> = { original: null, high: 160, low: 96 };

/**
 * What "Automatic" means in this browser, said plainly. Firefox and Safari (so every iPhone
 * and iPad) have no Network Information API, so Automatic can only ever mean Original there;
 * the person has to choose High or Data saver themselves when on mobile data.
 */
export function autoNote(c: ConnectionInfo | null | undefined): string {
	return c
		? 'High on a mobile connection, Data saver when the browser is asked to save data, otherwise Original'
		: "Always Original in this browser, which can't tell mobile data from Wi-Fi. Choose High or Data saver yourself when you're on mobile data";
}

/** What the player engine is decoding: the stream as it actually arrives at this device. */
export interface StreamSource {
	/** The codec's display name from the engine ("FLAC", "Opus", "MP3", "PCM", …). */
	codec: string;
	lossless: boolean;
	/** Bit depth of a lossless stream, when the engine knows it. */
	bits: number | null;
	/** Channels, when the engine knows them. */
	channels?: number | null;
}

/**
 * The engine's `source` status field, checked: it crosses from the worker as plain data and
 * ends up on screen, so only a short, tidy codec name and sane numbers are accepted.
 */
export function streamOf(raw: unknown): StreamSource | null {
	if (!raw || typeof raw !== 'object') return null;
	const r = raw as Record<string, unknown>;
	if (typeof r.codec !== 'string' || !/^[A-Za-z0-9][A-Za-z0-9 ()./-]{0,19}$/.test(r.codec)) return null;
	return { codec: r.codec, lossless: r.lossless === true, bits: whole(r.bits, 1, 64), channels: whole(r.channels, 1, 64) };
}

const whole = (v: unknown, lo: number, hi: number): number | null => (typeof v === 'number' && Number.isSafeInteger(v) && v >= lo && v <= hi ? v : null);
const plain = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object' && !Array.isArray(v);

/**
 * What LMS served, from the worker's status (the gateway's stream message, already checked
 * there). Checked again as it crosses to the page: a short lower-case codec token, numbers in
 * range; anything else makes it unknown.
 */
export function servedOf(raw: unknown): Served | null {
	if (!plain(raw)) return null;
	const codec = typeof raw.codec === 'string' && /^[a-z][a-z0-9]{0,11}$/.test(raw.codec) ? raw.codec : null;
	if (raw.codec !== null && raw.codec !== undefined && codec === null) return null;
	const out: Served = { codec, rate: whole(raw.rate, 1000, 10_000_000), bits: whole(raw.bits, 1, 64), channels: whole(raw.channels, 1, 64) };
	if (typeof raw.gain_db === 'number' && Number.isFinite(raw.gain_db) && Math.abs(raw.gain_db) <= 100) out.gain_db = raw.gain_db;
	return out;
}

/** What the gateway sent, from the worker's status: pass-through, or Opus at a level. */
export function sentOf(raw: unknown): Sent | null {
	if (!plain(raw)) return null;
	if (raw.mode === 'passthrough') return { mode: 'passthrough' };
	const kbps = whole(raw.kbps, 6, 1000);
	if (raw.mode === 'opus' && kbps !== null && (raw.level === 'high' || raw.level === 'low')) return { mode: 'opus', kbps, level: raw.level };
	return null;
}

/** The level a stream was sent at (passed through: Original; Opus at its level), or null when unknown. */
export function levelOfSent(raw: unknown): StreamQuality | null {
	const s = sentOf(raw);
	if (!s) return null;
	return s.mode === 'passthrough' ? 'original' : (s.level ?? null);
}

/** The gateway's echo of a level it has applied (`{"quality":…}`, checked by the worker): the level, or null. */
export function qualityEcho(raw: unknown): StreamQuality | null {
	return raw === 'original' || raw === 'high' || raw === 'low' ? raw : null;
}

/**
 * Why the level in force is the one in force, for the gateway block and the switch:
 * overridden from the signal path; chosen in Settings; or Automatic, which follows the
 * network where the browser reports it ("High, on mobile data", "Original, on Wi-Fi") and is
 * otherwise "chosen automatically" (Safari and Firefox can't tell mobile data from Wi-Fi).
 */
export function originOf(pref: QualityPreference, override: StreamQuality | null, c: ConnectionInfo | null | undefined): Origin {
	if (override) return { kind: 'override', level: override };
	if (pref !== 'auto') return { kind: 'setting', network: null, level: pref };
	const level = autoQuality(c);
	if (c?.saveData !== true && c?.type === 'cellular') return { kind: 'setting', network: 'mobile', level };
	if (c?.saveData !== true && c?.type === 'wifi') return { kind: 'setting', network: 'wifi', level };
	return { kind: 'automatic', level };
}

/** The level in force: an override from the signal path, else the preference's. */
export function levelInForce(pref: QualityPreference, override: StreamQuality | null, c: ConnectionInfo | null | undefined): StreamQuality {
	return override ?? effectiveQuality(pref, c);
}

/**
 * True when the gateway re-encoded this stream: it only ever turns FLAC into Opus for the
 * High and Data saver levels, so an Opus stream for a track that isn't Opus in the library
 * (or for an unknown track when a lower level was asked for) is its work.
 */
export function reencoded(trackCodec: string | null, stream: StreamSource | null, level: StreamQuality | null): boolean {
	if (!stream || stream.lossless || stream.codec !== 'Opus') return false;
	if (trackCodec) return trackCodec !== 'Opus';
	return level === 'high' || level === 'low';
}

const KEY = 'anywhere.stream.quality';
type Store = Pick<Storage, 'getItem' | 'setItem'>;
function safeLocal(): Store | undefined {
	try {
		return globalThis.localStorage;
	} catch {
		return undefined;
	}
}

export function sanitisePreference(v: unknown): QualityPreference {
	return PREFERENCES.includes(v as QualityPreference) ? (v as QualityPreference) : 'auto';
}

/** The preference kept in this browser ("Automatic" unless changed). */
export function loadPreference(store: Store | undefined = safeLocal()): QualityPreference {
	try {
		return sanitisePreference(store?.getItem(KEY));
	} catch {
		return 'auto';
	}
}

export function savePreference(p: QualityPreference, store: Store | undefined = safeLocal()) {
	try {
		store?.setItem(KEY, sanitisePreference(p));
	} catch {
		/* not essential */
	}
}

/** The browser's connection object, if it has one. */
export function connection(nav: unknown = globalThis.navigator): (ConnectionInfo & EventTarget) | null {
	const c = (nav as { connection?: unknown } | undefined)?.connection;
	return c && typeof c === 'object' ? (c as ConnectionInfo & EventTarget) : null;
}
