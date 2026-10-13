// Tappable credits (Wave 2 §3.1): every contributor role LMS reports for a track
// (`titles … tags:AS`), each name its own link to that contributor's albums in that role.
//
// LMS answers with one field per role (`composer: "A, B"`) and its ids (`composer_ids:
// "3,4"`). Names are joined with ", ", so a name that itself holds a comma can't be split
// safely: names and ids are only paired when the counts agree; otherwise the names are
// shown as plain text. Nothing here trusts the answer's shape: odd input gives fewer
// credits, never an exception.
import { ROLES, type Role } from './commands';
import { textOf } from './format';

/** Credits in the order they are listed. */
export const CREDIT_ORDER: readonly Role[] = ['ALBUMARTIST', 'ARTIST', 'TRACKARTIST', 'COMPOSER', 'CONDUCTOR', 'BAND'];

export const ROLE_LABELS: Record<Role, string> = {
	ALBUMARTIST: 'Album artist',
	ARTIST: 'Artist',
	TRACKARTIST: 'Track artist',
	COMPOSER: 'Composer',
	CONDUCTOR: 'Conductor',
	BAND: 'Band',
};

/** The field LMS answers with for each role. */
const FIELD: Record<Role, string> = {
	ALBUMARTIST: 'albumartist',
	ARTIST: 'artist',
	TRACKARTIST: 'trackartist',
	COMPOSER: 'composer',
	CONDUCTOR: 'conductor',
	BAND: 'band',
};

/** Most names listed for one role (a long cast is cut short rather than drawn forever). */
export const MAX_PEOPLE = 30;

export interface Person {
	name: string;
	/** LMS contributor id, or null when the name can't be tied to one. */
	id: number | null;
}
export interface Credit {
	role: Role;
	label: string;
	people: Person[];
}

function ids(v: unknown): number[] | null {
	if (typeof v === 'number') return Number.isSafeInteger(v) && v > 0 ? [v] : null;
	if (typeof v !== 'string' || v.length > 2000) return null;
	const parts = v.split(',').map((s) => s.trim());
	if (!parts.length || parts.some((p) => !/^\d{1,15}$/.test(p))) return null;
	const out = parts.map(Number);
	return out.every((n) => Number.isSafeInteger(n) && n > 0) ? out : null;
}

/** One role's people, paired with ids where that can be done safely. */
function people(name: unknown, idList: unknown): Person[] {
	const whole = textOf(name, 2000).trim();
	const known = ids(idList);
	if (!whole) return [];
	if (known && known.length === 1) return [{ name: whole, id: known[0]! }];
	const names = whole.split(', ').map((s) => s.trim());
	if (known && names.length === known.length && names.every(Boolean)) {
		return names.slice(0, MAX_PEOPLE).map((n, i) => ({ name: n, id: known[i]! }));
	}
	return [{ name: whole, id: null }];
}

/** The credits of one track (an entry of `titles_loop`), in `CREDIT_ORDER`, empty roles left out. */
export function parseCredits(track: unknown): Credit[] {
	if (!track || typeof track !== 'object' || Array.isArray(track)) return [];
	const t = track as Record<string, unknown>;
	const out: Credit[] = [];
	for (const role of CREDIT_ORDER) {
		const f = FIELD[role];
		const p = people(t[f], t[`${f}_ids`]);
		if (p.length) out.push({ role, label: ROLE_LABELS[role], people: p });
	}
	return out;
}

/** The URL word for a role (`#/artist/7/composer`). */
export function roleSlug(r: Role): string {
	return r.toLowerCase();
}

/** The role named by a URL word, or null. */
export function roleFromSlug(v: unknown): Role | null {
	if (typeof v !== 'string' || v.length > 20) return null;
	const up = v.toUpperCase();
	return (ROLES as readonly string[]).includes(up) ? (up as Role) : null;
}

/** In-app link to a contributor's albums in one role, or null for a bad id. */
export function roleLink(artist: unknown, r: Role): string | null {
	const n = typeof artist === 'number' ? artist : typeof artist === 'string' && /^\d{1,15}$/.test(artist) ? Number(artist) : NaN;
	if (!Number.isSafeInteger(n) || n <= 0) return null;
	return `#/artist/${n}/${roleSlug(r)}`;
}

/**
 * A contributor's name as LMS credits them in `role`, from a `roleTrack` answer: the name
 * paired with their id, or '' when it can't be told.
 */
export function nameInRole(result: unknown, artist: number, role: Role): string {
	const loop = (result as { titles_loop?: unknown } | null)?.titles_loop;
	if (!Array.isArray(loop)) return '';
	for (const t of loop.slice(0, 5)) {
		for (const c of parseCredits(t)) {
			if (c.role !== role) continue;
			const p = c.people.find((x) => x.id === artist);
			if (p) return p.name;
		}
	}
	return '';
}
