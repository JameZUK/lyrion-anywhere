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
const SORTS: readonly AlbumSort[] = ['album', 'artistalbum', 'yearalbum', 'new', 'random'];

/** LMS contributor roles (the gateway passes on only these). */
export const ROLES = ['ALBUMARTIST', 'ARTIST', 'COMPOSER', 'CONDUCTOR', 'BAND', 'TRACKARTIST'] as const;
export type Role = (typeof ROLES)[number];
export function role(v: unknown): Role {
	if (typeof v !== 'string' || !(ROLES as readonly string[]).includes(v)) throw new RangeError('bad role');
	return v as Role;
}

/** An LMS virtual library id: short hex (the gateway refuses anything else). */
export function libraryId(v: unknown): string {
	if (typeof v !== 'string' || !/^[0-9a-f]{1,32}$/i.test(v)) throw new RangeError('bad library id');
	return v;
}

/** A year as LMS lists them: 0 ("no year") up to four digits. */
export function year(v: unknown): number {
	const n = typeof v === 'number' ? v : typeof v === 'string' && /^\d{1,4}$/.test(v) ? Number(v) : NaN;
	if (!Number.isInteger(n) || n < 0 || n > 9999) throw new RangeError(`bad year: ${String(v).slice(0, 40)}`);
	return n;
}

/** What an album or artist list is narrowed to. Every value is checked before use. */
export interface Filter {
	genre?: number;
	year?: number;
	artist?: number;
	compilation?: boolean;
	role?: Role;
	/** A virtual library, or none for the whole library. */
	library?: string | null;
}

function filterParams(f: Filter, allowed: (keyof Filter)[]): string[] {
	const out: string[] = [];
	const has = (k: keyof Filter) => allowed.includes(k) && f[k] !== undefined && f[k] !== null;
	if (has('genre')) out.push(`genre_id:${id(f.genre)}`);
	if (has('year')) out.push(`year:${year(f.year)}`);
	if (has('artist')) out.push(`artist_id:${id(f.artist)}`);
	if (has('compilation')) out.push(`compilation:${f.compilation ? 1 : 0}`);
	if (has('role')) out.push(`role_id:${role(f.role)}`);
	if (has('library')) out.push(`library_id:${libraryId(f.library)}`);
	return out;
}

export const players = (): Command => ['players', 0, 100];
export const albums = (from: number, n: number, sort: AlbumSort = 'album', search?: string, filter: Filter = {}): Command => {
	if (!SORTS.includes(sort)) throw new RangeError('bad sort');
	return [
		'albums', start(from), count(n), ALBUM_TAGS, `sort:${sort}`, ...(search ? [`search:${text(search)}`] : []),
		...filterParams(filter, ['genre', 'year', 'artist', 'compilation', 'library']),
	];
};
export const artistAlbums = (artist: number): Command => ['albums', 0, 500, ALBUM_TAGS, 'sort:yearalbum', `artist_id:${id(artist)}`];
export const albumById = (album: number): Command => ['albums', 0, 1, ALBUM_TAGS, `album_id:${id(album)}`];
export const artists = (from: number, n: number, search?: string, filter: Filter = {}): Command => [
	'artists', start(from), count(n), ...(search ? [`search:${text(search)}`] : []),
	...filterParams(filter, ['genre', 'role', 'library']),
];
/** Does the library have any artists in this role? (One row is enough to tell.) */
export const hasRole = (r: Role, library?: string | null): Command => ['artists', 0, 1, ...filterParams({ role: r, library }, ['role', 'library'])];
export const genres = (from: number, n: number, library?: string | null, search?: string): Command => [
	'genres', start(from), count(n), ...(search ? [`search:${text(search)}`] : []), ...filterParams({ library }, ['library']),
];
export const genreById = (genre: number): Command => ['genres', 0, 1, `genre_id:${id(genre)}`];
export const years = (from: number, n: number, library?: string | null): Command => [
	'years', start(from), count(n), 'hasAlbums:1', ...filterParams({ library }, ['library']),
];
/** One folder of the music folder (the top level without an id). Never a path. */
export const musicFolder = (from: number, n: number, folder?: number): Command => [
	'musicfolder', start(from), count(n), ...(folder === undefined ? [] : [`folder_id:${id(folder)}`]), 'tags:cdt',
];
/** LMS's virtual libraries, if any. */
export const libraries = (): Command => ['libraries'];
/** Albums by id (Home shelves). LMS lists them in its own order: see `inOrder`. */
export const albumsByIds = (ids: readonly number[]): Command => {
	const list = ids.map(id).join(',');
	if (!list || list.length > ID_LIST_MAX) throw new RangeError('bad album list');
	return ['albums', 0, ids.length, ALBUM_TAGS, `album_id:${list}`];
};
/** Tracks by id (listening history). LMS lists them in its own order: see `inOrder`. */
export const tracksByIds = (ids: readonly number[]): Command => {
	const list = ids.map(id).join(',');
	if (!list || list.length > ID_LIST_MAX) throw new RangeError('bad track list');
	return ['titles', 0, ids.length, TRACK_TAGS, `track_id:${list}`];
};
/** Items put back in the order of `ids` (unknown ids dropped, extras left out). */
export function inOrder<T extends { id?: unknown }>(items: readonly T[], ids: readonly number[]): T[] {
	const by = new Map<number, T>();
	for (const it of items) {
		const n = typeof it.id === 'number' ? it.id : typeof it.id === 'string' ? Number(it.id) : NaN;
		if (Number.isSafeInteger(n) && !by.has(n)) by.set(n, it);
	}
	return ids.map((i) => by.get(i)).filter((x): x is T => x !== undefined);
}
export const albumTracks = (album: number): Command => ['titles', 0, 500, TRACK_TAGS, 'sort:tracknum', `album_id:${id(album)}`];
export const search = (term: string, n = 20): Command => ['search', 0, count(n), `term:${text(term)}`];

/** LMS favourite / menu item ids, e.g. "f047f379.3". */
export function itemId(v: unknown): string {
	if (typeof v !== 'string' || !/^[A-Za-z0-9._-]{1,128}$/.test(v)) throw new RangeError('bad item id');
	return v;
}

export const favourites = (from: number, n: number, item?: string): Command => [
	'favorites', 'items', start(from), count(n), ...(item ? [`item_id:${itemId(item)}`] : []),
];
export const playlists = (from: number, n: number): Command => ['playlists', start(from), count(n)];
export const playlistTracks = (playlist: number): Command => [
	'playlists', 'tracks', 0, 500, `playlist_id:${id(playlist)}`, TRACK_TAGS,
];
export const newPlaylist = (name: string): Command => ['playlists', 'new', `name:${text(name) || 'New playlist'}`];
export const renamePlaylist = (playlist: number, name: string): Command => [
	'playlists', 'rename', `playlist_id:${id(playlist)}`, `newname:${text(name) || 'Playlist'}`,
];
export const deletePlaylist = (playlist: number): Command => ['playlists', 'delete', `playlist_id:${id(playlist)}`];
export const addToPlaylist = (playlist: number, track: number): Command => [
	'playlists', 'edit', `playlist_id:${id(playlist)}`, 'cmd:add', `track_id:${id(track)}`,
];
export const removeFromPlaylist = (playlist: number, index: number): Command => [
	'playlists', 'edit', `playlist_id:${id(playlist)}`, 'cmd:delete', `index:${start(index)}`,
];
export const moveInPlaylist = (playlist: number, from: number, to: number): Command => [
	'playlists', 'edit', `playlist_id:${id(playlist)}`, 'cmd:move', `index:${start(from)}`, `toindex:${start(to)}`,
];

/** In-app link to a year's albums, or null. */
export function yearLink(v: unknown): string | null {
	try {
		return `#/year/${year(v)}`;
	} catch {
		return null;
	}
}

// --- apps, radio and context menus (SlimBrowse) --------------------------------------------

/** An app or radio command word, as LMS lists them (`spotty`, `bbcsounds`, …). */
export function appCommand(v: unknown): string {
	if (typeof v !== 'string' || !/^[a-z0-9_]{1,64}$/.test(v)) throw new RangeError('bad app');
	return v;
}
export const appsList = (): Command => ['apps', 0, 100];
/** The player's home menu, where plugins add entries (filtered by the gateway). */
export const homeMenu = (): Command => ['menu', 0, 500, 'direct:1'];
export const radiosList = (): Command => ['radios', 0, 100];
/** An app's top menu, in menu (SlimBrowse) format. */
export function appMenu(app: string): (from: number, n: number) => Command {
	const a = appCommand(app); // validated now, not when the menu is fetched
	return (from, n) => [a, 'items', start(from), count(n), 'menu:1'];
}

export type InfoKind = 'track' | 'album' | 'artist' | 'genre' | 'playlist' | 'year' | 'folder';
/** The "More…" (context) menu for a library item. */
export function infoMenu(kind: InfoKind, item: number): (from: number, n: number) => Command {
	const sel = kind === 'year' ? `year:${year(item)}` : `${kind}_id:${id(item)}`;
	return (from, n) => [`${kind}info`, 'items', start(from), count(n), sel, 'menu:1'];
}

// --- playback (player commands) ----------------------------------------------------------

export const status = (): Command => ['status', '-', 1, STATUS_TAGS];
/** What a player is playing, in brief (for the player list). */
export const nowPlaying = (): Command => ['status', '-', 1, 'tags:al'];
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

// --- synchronisation (player groups) -----------------------------------------------------

/** All sync groups (server query; the gateway keeps only players the caller may control). */
export const syncGroups = (): Command => ['syncgroups', '?'];
/** Sent for the player that joins: it joins `other`'s group. */
export const syncWith = (other: string): Command => ['sync', playerId(other)];
/** Sent for the player that leaves its group. */
export const unsync = (): Command => ['sync', '-'];

/** What to do with a selection: replace the queue and play, play next, or append. */
export type Placement = 'play' | 'next' | 'add';
const PLACEMENT = { play: 'load', next: 'insert', add: 'add' } as const;

export type Selection =
	| { album: number }
	| { track: number }
	| { artist: number }
	| { playlist: number }
	| { genre: number }
	| { year: number }
	| { folder: number };

function selector(sel: Selection): string {
	if ('album' in sel) return `album_id:${id(sel.album)}`;
	if ('track' in sel) return `track_id:${id(sel.track)}`;
	if ('playlist' in sel) return `playlist_id:${id(sel.playlist)}`;
	if ('genre' in sel) return `genre_id:${id(sel.genre)}`;
	if ('year' in sel) return `year:${year(sel.year)}`;
	if ('folder' in sel) return `folder_id:${id(sel.folder)}`;
	return `artist_id:${id(sel.artist)}`;
}

export function enqueue(sel: Selection, placement: Placement, playIndex?: number): Command {
	const cmd: Command = ['playlistcontrol', `cmd:${PLACEMENT[placement]}`, selector(sel)];
	if (placement === 'play' && playIndex !== undefined) cmd.push(`play_index:${start(playIndex)}`);
	return cmd;
}

/** The gateway takes an id list of at most this many characters (`1,2,3`). */
export const ID_LIST_MAX = 200;

/**
 * Replace the queue with these tracks (`load`, starting at `playIndex`) or append them
 * (`add`). Every id is checked, and the list must fit the gateway's limit (see `chunkIds`).
 */
export function loadTracks(ids: readonly number[], how: 'load' | 'add', playIndex?: number): Command {
	const list = ids.map(id).join(',');
	if (!list || list.length > ID_LIST_MAX) throw new RangeError('bad track list');
	const cmd: Command = ['playlistcontrol', `cmd:${how}`, `track_id:${list}`];
	if (how === 'load' && playIndex !== undefined) cmd.push(`play_index:${start(playIndex)}`);
	return cmd;
}

/** Split ids into lists that each fit the gateway's limit, keeping their order. */
export function chunkIds(ids: readonly number[], max = ID_LIST_MAX): number[][] {
	const out: number[][] = [];
	let cur: number[] = [];
	let len = 0;
	for (const v of ids) {
		const n = String(id(v)).length;
		if (cur.length && len + 1 + n > max) {
			out.push(cur);
			cur = [];
			len = 0;
		}
		len += (cur.length ? 1 : 0) + n;
		cur.push(v);
	}
	if (cur.length) out.push(cur);
	return out;
}

// --- links -------------------------------------------------------------------------------

/** In-app link to an album or artist, or null if LMS gave no usable id (e.g. "12,34" → 12). */
export function link(kind: 'album' | 'artist' | 'playlist' | 'genre' | 'folder', v: unknown): string | null {
	const first = typeof v === 'string' ? v.split(',')[0] : v;
	try {
		return `#/${kind}/${id(first)}`;
	} catch {
		return null;
	}
}

/** Play, play next or add a favourite (radio stations, tracks, app items). */
export function playFavourite(item: string, placement: Placement): Command {
	const verb = { play: 'play', next: 'insert', add: 'add' }[placement];
	return ['favorites', 'playlist', verb, `item_id:${itemId(item)}`];
}

// --- artwork -----------------------------------------------------------------------------

/** `image`, `image_300x300`, `image_300x300_o`, optionally with an image extension. */
const PROXY_NAME = /^image(?:_\d{1,4}x\d{1,4}(?:_[a-z])?)?(?:\.(?:jpe?g|png|gif|webp))?$/;

/** A well-formed percent-encoded segment that decodes to an http(s) URL (as the gateway requires). */
function proxySegment(seg: string): boolean {
	if (!/^(?:[A-Za-z0-9._~!$&'()*+,;=:@-]|%[0-9A-Fa-f]{2})+$/.test(seg)) return false;
	let url: string;
	try {
		url = decodeURIComponent(seg);
	} catch {
		return false;
	}
	// Control characters or spaces in the URL: refuse (the gateway does too).
	return /^https?:\/\/[^\s\\]+$/.test(url) && !/[\u0000-\u001f\u007f]/.test(url);
}

/**
 * Path (for Api.art) of an icon or image LMS named in a menu (favourites, apps), or null.
 * Only the forms the gateway relays: its own icons, plugin icons and image-proxy URLs.
 */
export function imagePath(image: unknown, size = 300): string | null {
	if (typeof image !== 'string' || image.length > 2048) return null;
	const px = Math.max(50, Math.min(1000, Math.round(size)));
	if (/^https?:\/\//i.test(image)) return `imageproxy/${encodeURIComponent(image)}/image_${px}x${px}`;
	const p = image.replace(/^\/+/, '');
	if (p.includes('\\') || p.includes('//') || /[?#\s]/.test(p)) return null;
	// Image proxy: exactly one percent-encoded http(s) URL segment, then the image name.
	// That segment is the only place a '%' may appear.
	const proxy = p.match(/^imageproxy\/([^/]+)\/([^/]+)$/);
	if (proxy) return proxySegment(proxy[1]!) && PROXY_NAME.test(proxy[2]!) ? p : null;
	if (p.includes('%') || p.includes('..') || p.split('/').some((s) => s === '.')) return null;
	if (/^(html\/images|plugins\/cache\/icons|plugins\/[A-Za-z0-9_]+\/html)\/[A-Za-z0-9._\/-]+\.(png|jpe?g|gif|svg)$/i.test(p)) return p;
	return null;
}

/** Path (for Api.art) of a cover by LMS cover id or track id; null if there is none. */
export function coverPath(cover: unknown, size = 300): string | null {
	if (typeof cover === 'number' && Number.isSafeInteger(cover) && cover > 0) cover = String(cover);
	if (typeof cover !== 'string' || !/^(?:\d{1,15}|[0-9a-f]{8,40})$/i.test(cover)) return null;
	const px = Math.max(50, Math.min(1000, Math.round(size)));
	return `music/${cover}/cover_${px}x${px}`;
}

/**
 * The commands for a seek that also starts playback, as the user expects from clicking the
 * position bar of a track that isn't playing:
 * - playing: just the seek;
 * - paused: the seek, then resume (LMS keeps a paused player paused after a seek);
 * - stopped (or unknown): start, then seek (LMS ignores a seek on a stopped player).
 */
export function seekAndPlay(mode: unknown, seconds: number): Command[] {
	if (mode === 'play') return [seek(seconds)];
	if (mode === 'pause') return [seek(seconds), pause(false)];
	return [play(), seek(seconds)];
}
