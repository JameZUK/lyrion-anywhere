// Artist biographies and photos, and album reviews (docs/plans/ux-research.md X5 part 1):
// the gateway's answers to GET /v1/info/artist/{id} and GET /v1/info/album/{id} (cleaned
// there from the Music & Artist Information plugin), checked again here as untrusted input.
// Text is only ever shown as text, never as markup.
import { imagePath } from '#lib/lms/commands';

/** Longest text accepted, in characters (the gateway keeps to the same limit). */
export const MAX_TEXT = 20_000;
/** Longest photo credit accepted. */
export const MAX_CREDITS = 120;

export interface ArtistInfo {
	/** Plain text; paragraphs separated by a blank line. */
	biography: string | null;
	/** An artwork path for Api.art (an image-proxy path, sized), or null. */
	photo: string | null;
	/** Where the photo comes from ("Discogs"), only with a photo. */
	credits: string | null;
}

export interface AlbumInfo {
	review: string | null;
}

export const NO_ARTIST_INFO: ArtistInfo = { biography: null, photo: null, credits: null };
export const NO_ALBUM_INFO: AlbumInfo = { review: null };

/** The gateway path for an artist's or album's information; null unless `id` is a positive integer. */
export function infoPath(kind: 'artist' | 'album', id: unknown): string | null {
	return typeof id === 'number' && Number.isSafeInteger(id) && id > 0 && id <= 999_999_999_999 ? `/v1/info/${kind}/${id}` : null;
}

// Control characters other than the line break, and invisible formatting characters
// (bidirectional overrides, zero-width characters, the byte order mark).
const HIDDEN = /[\u0000-\u0009\u000b-\u001f\u007f-\u009f\u200b-\u200f\u202a-\u202e\u2060-\u2069\ufeff]/g;

/** Untrusted text made safe to show: hidden characters removed, lines trimmed, at most one blank line in a row, capped. */
export function plainText(v: unknown, max = MAX_TEXT): string | null {
	if (typeof v !== 'string') return null;
	const lines = v.slice(0, max * 2).replace(/\r\n?/g, '\n').replace(/\t/g, ' ').replace(HIDDEN, '').split('\n');
	const kept: string[] = [];
	for (const raw of lines) {
		const l = raw.replace(/\s+/g, ' ').trim();
		if (!l && (!kept.length || !kept[kept.length - 1])) continue;
		kept.push(l);
	}
	while (kept.length && !kept[kept.length - 1]) kept.pop();
	const out = [...kept.join('\n')].slice(0, max).join('').trim();
	return out || null;
}

/**
 * The artwork path (for Api.art) of the gateway's photo path, sized: only its own image-proxy
 * paths (`/v1/art/imageproxy/<encoded URL>/image`), checked as every other image path is.
 */
export function photoPath(v: unknown, size = 600): string | null {
	if (typeof v !== 'string' || v.length > 4096) return null;
	const m = v.match(/^\/v1\/art\/(imageproxy\/[^/?#\s]+)\/image$/);
	if (!m || imagePath(`${m[1]}/image`) === null) return null;
	const px = Math.max(50, Math.min(1000, Math.round(size)));
	return imagePath(`${m[1]}/image_${px}x${px}`);
}

const record = (v: unknown): Record<string, unknown> => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : {});

/** The gateway's artist answer, checked. */
export function sanitiseArtistInfo(v: unknown, size = 600): ArtistInfo {
	const o = record(v);
	const photo = photoPath(o.photo, size);
	const credits = photo ? (plainText(o.credits, MAX_CREDITS)?.replace(/\n+/g, ' ') ?? null) : null;
	return { biography: plainText(o.biography), photo, credits };
}

/** The gateway's album answer, checked. */
export function sanitiseAlbumInfo(v: unknown): AlbumInfo {
	return { review: plainText(record(v).review) };
}

/** Paragraphs of cleaned text (separated by blank lines). */
export function paragraphs(text: string | null | undefined): string[] {
	return (text ?? '').split(/\n{2,}/).filter((p) => p.trim());
}

/** Is the text longer than its collapsed view (about three lines)? */
export function isLong(text: string | null | undefined, chars = 240): boolean {
	const p = paragraphs(text);
	return p.length > 1 || (p[0]?.length ?? 0) > chars || (p[0]?.split('\n').length ?? 0) > 3;
}
