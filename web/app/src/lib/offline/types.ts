// Offline downloads: the server contract (GET /v1/download/track/{id}/info and
// GET /v1/download/track/{id}?format=) and what is kept on the device for each track.
//
// Everything the server says about a track is checked here before it is used or stored:
// the download URL is built only from a validated positive integer id and a format from a
// fixed list, never from strings the server (or library tags) supplied.

export const FORMATS = ['original', 'mp3', 'opus'] as const;
export type Format = (typeof FORMATS)[number];

export const FORMAT_LABELS: Record<Format, string> = { original: 'Original', mp3: 'MP3', opus: 'Opus' };

export const isFormat = (v: unknown): v is Format => typeof v === 'string' && (FORMATS as readonly string[]).includes(v);

/** A track id from LMS: a positive safe integer (numbers only, or a string of digits). */
export function trackId(v: unknown): number {
	const n = typeof v === 'number' ? v : typeof v === 'string' && /^\d{1,15}$/.test(v) ? Number(v) : NaN;
	if (!Number.isSafeInteger(n) || n <= 0) throw new RangeError('bad track id');
	return n;
}

export const infoUrl = (id: number) => `/v1/download/track/${trackId(id)}/info`;

export function downloadUrl(id: number, format: Format): string {
	if (!isFormat(format)) throw new RangeError('bad format');
	return `/v1/download/track/${trackId(id)}?format=${format}`;
}

export interface TrackInfo {
	id: number;
	/** Stable across LMS rescans (track ids are not): the storage key. */
	key: string;
	title: string;
	artist: string;
	album: string;
	albumId: number | null;
	tracknum: number | null;
	duration: number;
	original: { format: string; contentType: string; size: number; ext: string };
	formats: Format[];
}

/** The server sent track details that don't fit the contract. */
export class BadInfo extends Error {
	constructor(what: string) {
		super(`unexpected track details (${what})`);
		this.name = 'BadInfo';
	}
}

const text = (v: unknown, max = 300) => (typeof v === 'string' ? (v.length > max ? `${v.slice(0, max - 1)}…` : v) : '');
const count = (v: unknown) => (typeof v === 'number' && Number.isSafeInteger(v) && v >= 0 ? v : null);
const positive = (v: unknown) => (typeof v === 'number' && Number.isSafeInteger(v) && v > 0 ? v : null);

/** A MIME type for audio, or '' (it becomes a Blob's type, so keep it plain). */
export function audioType(v: unknown): string {
	return typeof v === 'string' && /^audio\/[a-z0-9.+-]{1,40}(; ?codecs=[a-z0-9.]{1,20})?$/i.test(v) ? v.toLowerCase() : '';
}

/** Content types for the transcodes: fixed, whatever the server says. */
export const TRANSCODE_TYPES: Record<Exclude<Format, 'original'>, { type: string; ext: string }> = {
	mp3: { type: 'audio/mpeg', ext: 'mp3' },
	opus: { type: 'audio/ogg; codecs=opus', ext: 'opus' },
};

export function parseInfo(raw: unknown, expectId: number): TrackInfo {
	if (!raw || typeof raw !== 'object' || Array.isArray(raw)) throw new BadInfo('not an object');
	const r = raw as Record<string, unknown>;
	if (r.id !== expectId) throw new BadInfo('id');
	if (typeof r.key !== 'string' || !/^[0-9a-f]{64}$/.test(r.key)) throw new BadInfo('key');
	const o = r.original;
	if (!o || typeof o !== 'object' || Array.isArray(o)) throw new BadInfo('original');
	const orig = o as Record<string, unknown>;
	// LMS doesn't always know a file's size (the server then sends null): 0 means unknown,
	// and the download's Content-Length is used instead. Anything else odd is refused.
	if (orig.size !== null && orig.size !== undefined && count(orig.size) === null) throw new BadInfo('size');
	const size = count(orig.size) ?? 0;
	const formats = Array.isArray(r.formats) ? [...new Set(r.formats.filter(isFormat))] : [];
	if (!formats.length) throw new BadInfo('formats');
	const duration = typeof r.duration === 'number' && Number.isFinite(r.duration) && r.duration > 0 ? r.duration : 0;
	return {
		id: expectId,
		key: r.key,
		title: text(r.title),
		artist: text(r.artist),
		album: text(r.album),
		albumId: positive(r.album_id),
		tracknum: count(r.tracknum),
		duration,
		original: {
			format: text(orig.format, 20).replace(/[^a-z0-9]/gi, '').toLowerCase(),
			contentType: audioType(orig.content_type),
			size,
			ext: typeof orig.ext === 'string' && /^[a-z0-9]{1,8}$/i.test(orig.ext) ? orig.ext.toLowerCase() : '',
		},
		formats,
	};
}

/** The format to fetch: the one asked for if the server offers it, else Original, else the first offered. */
export function pickFormat(want: Format, offered: readonly Format[]): Format {
	if (offered.includes(want)) return want;
	if (offered.includes('original')) return 'original';
	return offered[0] ?? 'original';
}

/** A downloaded track, as stored on this device (no tokens, ever). */
export interface StoredTrack {
	/** The signed-in user it belongs to: downloads are kept apart per user. */
	user: string;
	key: string;
	/** The LMS track id when it was downloaded (may change on a rescan; refreshed on re-download). */
	id: number;
	title: string;
	artist: string;
	album: string;
	albumId: number | null;
	tracknum: number | null;
	duration: number;
	format: Format;
	/** For originals, the file type (e.g. "flc"); for transcodes, the format. */
	codec: string;
	/**
	 * The library file's type (e.g. "flc"), so a transcode's quality badge can say it was
	 * transcoded. Missing on tracks stored before it was kept.
	 */
	source?: string;
	ext: string;
	size: number;
	contentType: string;
	downloadedAt: number;
	/** How many stored pieces the audio is in. */
	chunks: number;
	/**
	 * Why it is on the device: the downloads that need it (see [`isRef`]). It is deleted only
	 * once none is left. Missing on tracks stored before groups existed: see [`refsOf`].
	 */
	refs?: string[];
}

/**
 * What a downloaded track was downloaded as part of: `album:<id>` (an album, or an
 * artist's albums), `playlist:<id>`, or `track` (on its own). Several downloads can need
 * the same track; it stays until the last one is removed.
 */
export type Ref = string;
const REF = /^(?:(?:album|playlist):[1-9]\d{0,14}|track)$/;
export const isRef = (v: unknown): v is Ref => typeof v === 'string' && REF.test(v);
export const albumRef = (id: number): Ref => `album:${trackId(id)}`;
export const playlistRef = (id: number): Ref => `playlist:${trackId(id)}`;
export const TRACK_REF: Ref = 'track';
/** At most this many refs on one track (a track in very many playlists keeps the first ones). */
export const MAX_REFS = 64;

/** Clean a list of refs: valid ones only, each once, bounded. */
export function cleanRefs(v: unknown): Ref[] {
	return Array.isArray(v) ? [...new Set(v.filter(isRef))].slice(0, MAX_REFS) : [];
}

/** A stored track's refs; tracks stored before refs existed belong to their album. */
export function refsOf(t: Pick<StoredTrack, 'refs' | 'albumId'>): Ref[] {
	if (Array.isArray(t.refs)) return cleanRefs(t.refs);
	return [t.albumId !== null && Number.isSafeInteger(t.albumId) && t.albumId > 0 ? `album:${t.albumId}` : TRACK_REF];
}

/** A playlist (or other named group) downloaded to this device: its name and track order. */
export interface StoredGroup {
	user: string;
	/** A [`Ref`] other than `track`. */
	id: Ref;
	name: string;
	/** LMS track ids in the playlist's order when it was downloaded. */
	ids: number[];
	savedAt: number;
}

/** Bytes per second for the transcodes (Opus 160 kb/s, MP3 320 kb/s CBR) and, when a file's
 *  size is unknown, for an original (CD-quality PCM: a FLAC is smaller, hi-res larger). */
export const BYTES_PER_SECOND: Record<Format, number> = { opus: 20_000, mp3: 40_000, original: 176_400 };
/** Assumed length of a track whose duration is unknown. */
export const FALLBACK_SECONDS = 300;

/**
 * How many bytes a download will take, for the storage limit: an original's file size when
 * LMS said it, else its duration at the format's bit rate. Never negative or NaN.
 */
export function estimateBytes(t: { duration?: unknown; filesize?: unknown }, format: Format): number {
	if (format === 'original') {
		const n = typeof t.filesize === 'number' ? t.filesize : typeof t.filesize === 'string' && /^\d{1,15}$/.test(t.filesize) ? Number(t.filesize) : NaN;
		if (Number.isSafeInteger(n) && n > 0) return n;
	}
	const d = Number(t.duration);
	const seconds = Number.isFinite(d) && d > 0 ? Math.min(d, 86_400) : FALLBACK_SECONDS;
	return Math.ceil(seconds * BYTES_PER_SECOND[format]);
}

/** A download that has started but not finished: what has been stored so far. */
export interface PartialDownload {
	user: string;
	key: string;
	format: Format;
	/** Expected size in bytes (0 if unknown). */
	total: number;
	received: number;
	chunks: number;
	contentType: string;
}

/** A download waiting in the queue, kept so it carries on after the page reloads. */
export interface QueuedDownload {
	id: number;
	albumId: number | null;
	choice: Format | 'auto';
	paused: boolean;
	/** The downloads that asked for it (missing in queues saved before groups). */
	refs?: Ref[];
	/** "Download now": allowed on mobile data although Wi-Fi only is on. */
	cellular?: boolean;
	/** Expected size, for the storage limit. */
	estimate?: number;
	/** The stable key, once the server has said it (so a cancel finds the stored bytes). */
	key?: string;
}

/** A play made offline, waiting to be sent to the server (POST /v1/plays). */
export interface QueuedPlay {
	user: string;
	/** Also the idempotency key the server dedupes by: `<start, 11 digits>-<uuid>`. */
	id: string;
	trackId: number;
	/** The download's stable key: the server checks it is still the same file. */
	trackKey: string;
	/** When playback started (Unix seconds). */
	playedAt: number;
	/** Seconds actually heard. */
	seconds: number;
}
