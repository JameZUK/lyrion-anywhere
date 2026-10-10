// LMS command builders, limited to what the gateway's allow-list accepts
// (docs/PROTOCOL.md). Ids are checked here so bad input never leaves the browser.
import type { Command } from '$lib/api/rpc';

export const STATUS_TAGS = 'tags:aAlcdeJKjNoqrsTIyx';
export const QUEUE_TAGS = 'tags:alcdJKj';
export const ALBUM_TAGS = 'tags:aljyS';
export const TRACK_TAGS = 'tags:adtlcJKj';

/** LMS library ids are positive integers. */
export function id(v: unknown): number {
	const n = typeof v === 'number' ? v : typeof v === 'string' && /^\d{1,15}$/.test(v) ? Number(v) : NaN;
	if (!Number.isSafeInteger(n) || n <= 0) throw new RangeError(`bad id: ${String(v).slice(0, 40)}`);
	return n;
}

/** LMS player ids look like MAC addresses. */
export function playerId(v: string): string {
	if (!/^[0-9a-f]{2}(:[0-9a-f]{2}){5}$/i.test(v)) throw new RangeError('bad player id');
	return v.toLowerCase();
}

const count = (n: number) => Math.max(1, Math.min(500, Math.floor(n)));
const start = (n: number) => Math.max(0, Math.floor(n));
/** Text parameters: printable, at most 200 characters (the gateway allows 256 per argument). */
export function text(s: string): string {
	const t = s.replace(/[\u0000-\u001f\u007f]/g, ' ').trim();
	return t.slice(0, 200);
}

// --- library -----------------------------------------------------------------------------

export type AlbumSort = 'album' | 'artistalbum' | 'yearalbum' | 'new' | 'random';

export const players = (): Command => ['players', 0, 100];
export const albums = (from: number, n: number, sort: AlbumSort = 'album', search?: string): Command => [
	'albums', start(from), count(n), ALBUM_TAGS, `sort:${sort}`, ...(search ? [`search:${text(search)}`] : []),
];
export const artistAlbums = (artist: number): Command => ['albums', 0, 500, ALBUM_TAGS, 'sort:yearalbum', `artist_id:${id(artist)}`];
export const albumById = (album: number): Command => ['albums', 0, 1, ALBUM_TAGS, `album_id:${id(album)}`];
export const artists = (from: number, n: number, search?: string): Command => [
	'artists', start(from), count(n), ...(search ? [`search:${text(search)}`] : []),
];
export const albumTracks = (album: number): Command => ['titles', 0, 500, TRACK_TAGS, 'sort:tracknum', `album_id:${id(album)}`];
export const search = (term: string, n = 20): Command => ['search', 0, count(n), `term:${text(term)}`];

// --- playback (player commands) ----------------------------------------------------------

export const status = (): Command => ['status', '-', 1, STATUS_TAGS];
export const queue = (from: number, n: number): Command => ['status', start(from), count(n), QUEUE_TAGS];
export const play = (): Command => ['play'];
export const pause = (on?: boolean): Command => (on === undefined ? ['pause'] : ['pause', on ? 1 : 0]);
export const stop = (): Command => ['stop'];
export const skip = (by: 1 | -1): Command => ['playlist', 'index', by > 0 ? '+1' : '-1'];
export const jump = (index: number): Command => ['playlist', 'index', start(index)];
export const seek = (seconds: number): Command => ['time', Math.max(0, Math.round(seconds * 10) / 10)];
export const volume = (v: number): Command => ['mixer', 'volume', Math.max(0, Math.min(100, Math.round(v)))];
export const mute = (): Command => ['mixer', 'muting', 'toggle'];
export const power = (on: boolean): Command => ['power', on ? 1 : 0];
export const removeFromQueue = (index: number): Command => ['playlist', 'delete', start(index)];
export const moveInQueue = (from: number, to: number): Command => ['playlist', 'move', start(from), start(to)];
export const clearQueue = (): Command => ['playlist', 'clear'];
export const shuffle = (mode: 0 | 1 | 2): Command => ['playlist', 'shuffle', mode];
export const repeat = (mode: 0 | 1 | 2): Command => ['playlist', 'repeat', mode];

/** What to do with a selection: replace the queue and play, play next, or append. */
export type Placement = 'play' | 'next' | 'add';
const PLACEMENT = { play: 'load', next: 'insert', add: 'add' } as const;

export type Selection = { album: number } | { track: number } | { artist: number };

export function enqueue(sel: Selection, placement: Placement, playIndex?: number): Command {
	const [key, value] =
		'album' in sel ? ['album_id', sel.album] : 'track' in sel ? ['track_id', sel.track] : ['artist_id', sel.artist];
	const cmd: Command = ['playlistcontrol', `cmd:${PLACEMENT[placement]}`, `${key}:${id(value)}`];
	if (placement === 'play' && playIndex !== undefined) cmd.push(`play_index:${start(playIndex)}`);
	return cmd;
}

// --- links -------------------------------------------------------------------------------

/** In-app link to an album or artist, or null if LMS gave no usable id (e.g. "12,34" → 12). */
export function link(kind: 'album' | 'artist', v: unknown): string | null {
	const first = typeof v === 'string' ? v.split(',')[0] : v;
	try {
		return `#/${kind}/${id(first)}`;
	} catch {
		return null;
	}
}

// --- artwork -----------------------------------------------------------------------------

/** Path (for Api.art) of a cover by LMS cover id or track id; null if there is none. */
export function coverPath(cover: unknown, size = 300): string | null {
	if (typeof cover === 'number' && Number.isSafeInteger(cover) && cover > 0) cover = String(cover);
	if (typeof cover !== 'string' || !/^(?:\d{1,15}|[0-9a-f]{8,40})$/i.test(cover)) return null;
	const px = Math.max(50, Math.min(1000, Math.round(size)));
	return `music/${cover}/cover_${px}x${px}`;
}
