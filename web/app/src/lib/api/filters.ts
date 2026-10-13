// Smart filters (docs/plans/wave2-design.md §5.6 › The contract): saved per user on the
// gateway, each a set of rules that all must match, producing albums. These checks mirror the
// contract, so a filter that fails here is never sent; the gateway checks everything again.
import { ROLES, type AlbumSort, type Role } from '$lib/lms/commands';

export const FILTER_SORTS = ['album', 'artistalbum', 'yearalbum', 'new', 'random'] as const satisfies readonly AlbumSort[];
export type FilterSort = (typeof FILTER_SORTS)[number];

export const FILTER_FIELDS = ['genre', 'decade', 'artist', 'compilation'] as const;
export type FilterField = (typeof FILTER_FIELDS)[number];

export type FilterRule =
	| { field: 'genre'; in: number[] }
	| { field: 'decade'; is: number }
	| { field: 'artist'; is: number; role?: Role }
	| { field: 'compilation'; is: boolean };

/** What is saved (and sent): everything but the id, which only the gateway gives. */
export interface FilterSpec {
	name: string;
	rules: FilterRule[];
	sort: FilterSort;
	limit: number;
}

export interface SmartFilter extends FilterSpec {
	id: number;
}

export const FILTER_LIMITS = {
	minRules: 1,
	maxRules: 8,
	nameMax: 64,
	genresMin: 1,
	genresMax: 20,
	decadeMin: 1900,
	decadeMax: 2090,
	limitMin: 1,
	limitMax: 500,
	/** The body, as sent. */
	maxBytes: 4096,
	maxFilters: 50,
	/** Albums asked for at once (`n`). */
	pageMax: 100,
	/** Albums played or shuffled from a filter. */
	playMax: 50,
} as const;

export const DECADES: readonly number[] = Array.from({ length: (FILTER_LIMITS.decadeMax - FILTER_LIMITS.decadeMin) / 10 + 1 }, (_, i) => FILTER_LIMITS.decadeMin + i * 10);

export const SORT_LABELS: Record<FilterSort, string> = {
	album: 'A–Z',
	artistalbum: 'Artist',
	yearalbum: 'Year',
	new: 'Recently added',
	random: 'Random',
};

export const FIELD_LABELS: Record<FilterField, string> = {
	genre: 'Genre',
	decade: 'Decade',
	artist: 'Artist',
	compilation: 'Compilation',
};

const MAX_ID = 999_999_999_999;
/** An id the server gave: a positive safe integer. */
export const validId = (v: unknown): v is number => typeof v === 'number' && Number.isSafeInteger(v) && v >= 1 && v <= MAX_ID;

/**
 * A name as the gateway keeps it (as device names are cleaned): letters of any script,
 * digits, spaces and `. , - _ ' ’ ( ) &`, with combining accents after something; anything
 * else is dropped, runs of whitespace become one space, and it is cut to 64 characters.
 * Empty when nothing usable is left.
 */
export function cleanFilterName(raw: string): string {
	let out = '';
	let count = 0;
	let space = false;
	for (const c of String(raw)) {
		if (/\s/u.test(c)) {
			space = count > 0;
			continue;
		}
		const combining = c >= '̀' && c <= 'ͯ';
		const ok = /[\p{Alphabetic}\p{N}]/u.test(c) || ".,-_'’()&".includes(c) || (combining && count > 0);
		if (!ok) continue;
		const need = space && !combining ? 2 : 1;
		if (count + need > FILTER_LIMITS.nameMax) break;
		if (space && !combining) {
			out += ' ';
			count++;
		}
		space = false;
		out += c;
		count++;
	}
	return out;
}

const isObject = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object' && !Array.isArray(v);
const onlyKeys = (o: Record<string, unknown>, keys: readonly string[]) => Object.keys(o).every((k) => keys.includes(k));

/** Why this rule isn't acceptable, or null. */
export function ruleProblem(r: unknown): string | null {
	if (!isObject(r)) return 'each rule is an object';
	switch (r.field) {
		case 'genre': {
			if (!onlyKeys(r, ['field', 'in'])) return 'unexpected field in a genre rule';
			const list = r.in;
			if (!Array.isArray(list) || list.length < FILTER_LIMITS.genresMin) return 'choose at least one genre';
			if (list.length > FILTER_LIMITS.genresMax) return `choose at most ${FILTER_LIMITS.genresMax} genres`;
			if (!list.every(validId)) return 'bad genre';
			if (new Set(list).size !== list.length) return 'a genre appears twice';
			return null;
		}
		case 'decade':
			if (!onlyKeys(r, ['field', 'is'])) return 'unexpected field in a decade rule';
			if (typeof r.is !== 'number' || !Number.isInteger(r.is) || r.is % 10 !== 0 || r.is < FILTER_LIMITS.decadeMin || r.is > FILTER_LIMITS.decadeMax) return 'choose a decade';
			return null;
		case 'artist':
			if (!onlyKeys(r, ['field', 'is', 'role'])) return 'unexpected field in an artist rule';
			if (!validId(r.is)) return 'choose an artist';
			if (r.role !== undefined && !(ROLES as readonly unknown[]).includes(r.role)) return 'bad role';
			return null;
		case 'compilation':
			if (!onlyKeys(r, ['field', 'is'])) return 'unexpected field in a compilation rule';
			if (typeof r.is !== 'boolean') return 'compilation is yes or no';
			return null;
		default:
			return 'unknown rule';
	}
}

/** Why this filter (without its id) isn't acceptable, or null. */
export function filterProblem(f: unknown): string | null {
	if (!isObject(f)) return 'a filter is an object';
	if (!onlyKeys(f, ['name', 'rules', 'sort', 'limit'])) return 'unexpected field in a filter';
	if (typeof f.name !== 'string' || !f.name || cleanFilterName(f.name) !== f.name) return 'give it a name (letters, digits and simple punctuation, up to 64 characters)';
	if (!Array.isArray(f.rules) || f.rules.length < FILTER_LIMITS.minRules) return 'add at least one rule';
	if (f.rules.length > FILTER_LIMITS.maxRules) return `at most ${FILTER_LIMITS.maxRules} rules`;
	const seen = new Set<unknown>();
	for (const r of f.rules) {
		const p = ruleProblem(r);
		if (p) return p;
		const field = (r as { field: unknown }).field;
		if (seen.has(field)) return 'each kind of rule can be used once';
		seen.add(field);
	}
	if (typeof f.sort !== 'string' || !(FILTER_SORTS as readonly string[]).includes(f.sort)) return 'unknown order';
	if (typeof f.limit !== 'number' || !Number.isInteger(f.limit) || f.limit < FILTER_LIMITS.limitMin || f.limit > FILTER_LIMITS.limitMax) {
		return `the limit is ${FILTER_LIMITS.limitMin} to ${FILTER_LIMITS.limitMax} albums`;
	}
	if (new TextEncoder().encode(JSON.stringify(f)).length > FILTER_LIMITS.maxBytes) return 'this filter is too big';
	return null;
}

/** A clean copy with only the contract's keys (the name cleaned), in a fixed order. */
export function cleanSpec(f: FilterSpec): FilterSpec {
	const rules = f.rules.map((r): FilterRule => {
		switch (r.field) {
			case 'genre':
				return { field: 'genre', in: [...r.in] };
			case 'decade':
				return { field: 'decade', is: r.is };
			case 'artist':
				return r.role === undefined ? { field: 'artist', is: r.is } : { field: 'artist', is: r.is, role: r.role };
			case 'compilation':
				return { field: 'compilation', is: r.is };
		}
	});
	return { name: cleanFilterName(f.name), rules, sort: f.sort, limit: f.limit };
}

/**
 * A filter from the gateway, or null if it isn't one (untrusted: only well-formed filters
 * with an id are shown). Unknown keys are ignored here; nothing read is sent back unchecked.
 */
export function filterFrom(v: unknown): SmartFilter | null {
	if (!isObject(v) || !validId(v.id)) return null;
	const spec = { name: v.name, rules: v.rules, sort: v.sort, limit: v.limit };
	if (filterProblem(spec)) return null;
	return { id: v.id, ...cleanSpec(spec as FilterSpec) };
}

/** A short description of a rule, for lists ("Genre: 2 chosen", "1990s"). */
export function ruleSummary(r: FilterRule, names: { genre?: (id: number) => string; artist?: (id: number) => string } = {}): string {
	switch (r.field) {
		case 'genre': {
			const n = r.in.map((id) => names.genre?.(id) ?? '').filter(Boolean);
			return n.length === r.in.length && n.length <= 3 ? n.join(', ') : `${r.in.length} ${r.in.length === 1 ? 'genre' : 'genres'}`;
		}
		case 'decade':
			return `${r.is}s`;
		case 'artist': {
			const who = names.artist?.(r.is) || 'An artist';
			return r.role ? `${who} (${r.role.toLowerCase()})` : who;
		}
		case 'compilation':
			return r.is ? 'Compilations only' : 'No compilations';
	}
}
