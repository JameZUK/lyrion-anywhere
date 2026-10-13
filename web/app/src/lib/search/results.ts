// Search results (Wave 2 §3.3), grouped by type: one LMS `search … extended:1` gives artists,
// albums and tracks (with their counts), and `playlists search:` gives playlists. LMS's answer
// is only displayed, never trusted: rows without a usable id are dropped, and odd shapes give
// fewer results rather than an exception.
import { textOf } from '$lib/lms/format';

export type HitKind = 'artist' | 'album' | 'track' | 'playlist';
export type SearchType = 'all' | 'artists' | 'albums' | 'tracks' | 'playlists';
export const SEARCH_TYPES: readonly SearchType[] = ['all', 'artists', 'albums', 'tracks', 'playlists'];
export const TYPE_LABELS: Record<SearchType, string> = {
	all: 'All',
	artists: 'Artists',
	albums: 'Albums',
	tracks: 'Tracks',
	playlists: 'Playlists',
};

export interface Hit {
	kind: HitKind;
	id: number;
	name: string;
	/** A second line: an album's artist, a track's artist and album. */
	sub: string;
	/** A cover id or track id for the artwork, if LMS gave one. */
	cover: string | number | null;
}

export interface Groups {
	artists: Hit[];
	albums: Hit[];
	tracks: Hit[];
	playlists: Hit[];
	/** How many of each LMS has in all (at least as many as listed). */
	counts: { artists: number; albums: number; tracks: number; playlists: number };
}

export const EMPTY_GROUPS: Groups = { artists: [], albums: [], tracks: [], playlists: [], counts: { artists: 0, albums: 0, tracks: 0, playlists: 0 } };

/** Most rows kept from one answer. */
const MAX_ROWS = 500;

function posId(v: unknown): number | null {
	const n = typeof v === 'number' ? v : typeof v === 'string' && /^\d{1,15}$/.test(v) ? Number(v) : NaN;
	return Number.isSafeInteger(n) && n > 0 ? n : null;
}

function cover(v: unknown): string | number | null {
	if (typeof v === 'number') return Number.isSafeInteger(v) && v > 0 ? v : null;
	return typeof v === 'string' && /^(?:\d{1,15}|[0-9a-f]{8,40})$/i.test(v) ? v : null;
}

const count = (v: unknown, listed: number) => {
	const n = typeof v === 'number' ? v : typeof v === 'string' && /^\d{1,9}$/.test(v) ? Number(v) : NaN;
	return Number.isSafeInteger(n) && n >= listed ? n : listed;
};

type Row = Record<string, unknown>;
const rows = (v: unknown): Row[] =>
	Array.isArray(v) ? (v.slice(0, MAX_ROWS).filter((r) => r && typeof r === 'object' && !Array.isArray(r)) as Row[]) : [];

/** Rows of one loop as hits: `id` and `name` name the fields holding them. */
export function hits(loop: unknown, kind: HitKind, id: string, name: string): Hit[] {
	const out: Hit[] = [];
	const seen = new Set<number>();
	for (const r of rows(loop)) {
		const n = posId(r[id]);
		const label = textOf(r[name]).trim();
		if (n === null || !label || seen.has(n)) continue;
		seen.add(n);
		const sub =
			kind === 'album'
				? textOf(r.artist)
				: kind === 'track'
					? [textOf(r.artist), textOf(r.album)].filter(Boolean).join(' · ')
					: '';
		out.push({ kind, id: n, name: label, sub, cover: kind === 'album' ? cover(r.artwork_track_id ?? r.artwork) : kind === 'track' ? cover(r.coverid ?? r.artwork_track_id) : null });
	}
	return out;
}

/** One `search … extended:1` answer and one `playlists search:` answer, grouped. */
export function groupResults(search: unknown, playlists?: unknown): Groups {
	const s = (search && typeof search === 'object' ? search : {}) as Row;
	const p = (playlists && typeof playlists === 'object' ? playlists : {}) as Row;
	const artists = hits(s.contributors_loop, 'artist', 'contributor_id', 'contributor');
	const albums = hits(s.albums_loop, 'album', 'album_id', 'album');
	const tracks = hits(s.tracks_loop, 'track', 'track_id', 'track');
	const lists = hits(p.playlists_loop, 'playlist', 'id', 'playlist');
	return {
		artists,
		albums,
		tracks,
		playlists: lists,
		counts: {
			artists: count(s.contributors_count, artists.length),
			albums: count(s.albums_count, albums.length),
			tracks: count(s.tracks_count, tracks.length),
			playlists: count(p.count, lists.length),
		},
	};
}

export function totalHits(g: Groups): number {
	return g.artists.length + g.albums.length + g.tracks.length + g.playlists.length;
}

/** Names compared as people read them: case, accents and spacing aside. */
export function normalise(s: string): string {
	return s
		.normalize('NFKD')
		.replace(/[̀-ͯ]/g, '')
		.toLocaleLowerCase('en-GB')
		.replace(/\s+/g, ' ')
		.trim();
}

/**
 * The Top result: an exact name match, or else one whose name starts with the term,
 * preferring an artist, then an album, then a track. None when nothing matches that well.
 */
export function topResult(g: Groups, term: string): Hit | null {
	const q = normalise(term);
	if (!q) return null;
	const order = [g.artists, g.albums, g.tracks];
	for (const test of [(n: string) => n === q, (n: string) => n.startsWith(q)]) {
		for (const list of order) {
			const hit = list.find((h) => test(normalise(h.name)));
			if (hit) return hit;
		}
	}
	return null;
}
