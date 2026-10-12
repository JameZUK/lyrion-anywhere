// Ratings and alarms (docs/plans/wave2-design.md §1–2): typed calls to the gateway, with
// answers checked as untrusted input.
import { bearer, callJson } from './http';
import type { Authed } from './me';

// --- ratings ---------------------------------------------------------------------------

export type RatingStyle = 'stars' | 'heart';

export interface TrackRating {
	track: number;
	/** The user's own rating, 0–100 (null: not rated). */
	mine: number | null;
	/** LMS's shared rating (Ratings Light or other apps). */
	shared: number | null;
	/** The blend shown: the user's own ×3, the shared ×1. */
	rating: number | null;
}

const rating = (v: unknown): number | null =>
	typeof v === 'number' && Number.isInteger(v) && v >= 0 && v <= 100 ? v : null;
const trackId = (v: unknown): number | null => (typeof v === 'number' && Number.isSafeInteger(v) && v > 0 ? v : null);

export function sanitiseRating(v: unknown): TrackRating | null {
	if (!v || typeof v !== 'object') return null;
	const o = v as Record<string, unknown>;
	const track = trackId(o.track);
	if (track === null) return null;
	return { track, mine: rating(o.mine), shared: rating(o.shared), rating: rating(o.rating) };
}

/** A rating to send: 0–100 in steps of 10 (half stars). */
export function ratingValue(v: number): number {
	if (!Number.isFinite(v)) throw new RangeError('bad rating');
	return Math.max(0, Math.min(100, Math.round(v / 10) * 10));
}

// --- alarms ----------------------------------------------------------------------------

export interface AlarmSound {
	id: string;
	category: string;
	title: string;
}

export interface Alarm {
	id: string;
	/** Seconds after midnight. */
	time: number;
	/** 0 (Sunday) – 6. */
	days: number[];
	enabled: boolean;
	repeat: boolean;
	volume: number | null;
	/** 0 off, 1 songs, 2 albums. */
	shuffle: 0 | 1 | 2;
	sound: { id: string | null; category: string; title: string } | null;
}

export interface AlarmListing {
	enabled: boolean;
	alarms: Alarm[];
	sounds: AlarmSound[];
	/** The new alarm's id, after adding. */
	id?: string;
}

export interface AlarmChange {
	time?: number;
	days?: number[];
	enabled?: boolean;
	repeat?: boolean;
	volume?: number;
	shuffle?: 0 | 1 | 2;
	sound?: string;
}

const PLAYER = /^[0-9a-f]{2}(:[0-9a-f]{2}){5}$/;
const ALARM_ID = /^[0-9a-f]{1,32}$/;
const SOUND_ID = /^(current|[0-9a-f]{16})$/;
const text = (v: unknown, max = 200) => (typeof v === 'string' ? v.slice(0, max) : '');

function player(p: string): string {
	const v = p.toLowerCase();
	if (!PLAYER.test(v)) throw new RangeError('bad player id');
	return v;
}

function sound(s: unknown): AlarmSound | null {
	if (!s || typeof s !== 'object') return null;
	const o = s as Record<string, unknown>;
	if (typeof o.id !== 'string' || !SOUND_ID.test(o.id)) return null;
	return { id: o.id, category: text(o.category, 20), title: text(o.title) };
}

function alarm(a: unknown): Alarm | null {
	if (!a || typeof a !== 'object') return null;
	const x = a as Record<string, unknown>;
	if (typeof x.id !== 'string' || !ALARM_ID.test(x.id)) return null;
	if (typeof x.time !== 'number' || !Number.isInteger(x.time) || x.time < 0 || x.time >= 86_400) return null;
	const days = Array.isArray(x.days) ? x.days.filter((d): d is number => Number.isInteger(d) && d >= 0 && d <= 6) : [];
	const s = x.sound && typeof x.sound === 'object' ? (x.sound as Record<string, unknown>) : null;
	return {
		id: x.id,
		time: x.time,
		days: [...new Set(days)].sort(),
		enabled: x.enabled === true,
		repeat: x.repeat !== false,
		volume: typeof x.volume === 'number' && x.volume >= 0 && x.volume <= 100 ? Math.round(x.volume) : null,
		shuffle: x.shuffle === 1 || x.shuffle === 2 ? x.shuffle : 0,
		sound: s
			? { id: typeof s.id === 'string' && SOUND_ID.test(s.id) ? s.id : null, category: text(s.category, 20), title: text(s.title) }
			: null,
	};
}

export function sanitiseAlarms(v: unknown): AlarmListing {
	const o = (v && typeof v === 'object' ? v : {}) as Record<string, unknown>;
	const sounds = (Array.isArray(o.sounds) ? o.sounds : []).slice(0, 500).map(sound).filter((s): s is AlarmSound => s !== null);
	const alarms = (Array.isArray(o.alarms) ? o.alarms : []).slice(0, 64).map(alarm).filter((a): a is Alarm => a !== null);
	const out: AlarmListing = { enabled: o.enabled !== false, alarms, sounds };
	if (typeof o.id === 'string' && ALARM_ID.test(o.id)) out.id = o.id;
	return out;
}

/** A change to send: only known fields, each checked. */
export function alarmBody(c: AlarmChange): AlarmChange {
	const out: AlarmChange = {};
	if (c.time !== undefined) {
		if (!Number.isInteger(c.time) || c.time < 0 || c.time >= 86_400) throw new RangeError('bad time');
		out.time = c.time;
	}
	if (c.days !== undefined) {
		const d = [...new Set(c.days)].sort();
		if (!d.length || d.some((x) => !Number.isInteger(x) || x < 0 || x > 6)) throw new RangeError('bad days');
		out.days = d;
	}
	if (c.enabled !== undefined) out.enabled = !!c.enabled;
	if (c.repeat !== undefined) out.repeat = !!c.repeat;
	if (c.volume !== undefined) out.volume = Math.max(0, Math.min(100, Math.round(c.volume)));
	if (c.shuffle !== undefined) {
		if (![0, 1, 2].includes(c.shuffle)) throw new RangeError('bad shuffle');
		out.shuffle = c.shuffle;
	}
	if (c.sound !== undefined) {
		if (!SOUND_ID.test(c.sound)) throw new RangeError('bad sound');
		out.sound = c.sound;
	}
	return out;
}

/** "07:00" ↔ seconds after midnight. */
export function timeText(secs: number): string {
	const h = Math.floor(secs / 3600);
	const m = Math.floor((secs % 3600) / 60);
	return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}`;
}
export function parseTime(t: string): number | null {
	const m = t.trim().match(/^(\d{1,2}):(\d{2})$/);
	if (!m) return null;
	const h = Number(m[1]);
	const min = Number(m[2]);
	return h < 24 && min < 60 ? h * 3600 + min * 60 : null;
}

export const DAY_NAMES = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'] as const;

/** "Weekdays", "Every day", "Weekends" or the days in order from Monday. */
export function daysText(days: number[]): string {
	const s = [...new Set(days)].sort().join(',');
	if (s === '0,1,2,3,4,5,6') return 'Every day';
	if (s === '1,2,3,4,5') return 'Weekdays';
	if (s === '0,6') return 'Weekends';
	return [1, 2, 3, 4, 5, 6, 0]
		.filter((d) => days.includes(d))
		.map((d) => DAY_NAMES[d])
		.join(' ');
}

/** The next time an enabled alarm in `alarms` rings after `now`, or null. */
export function nextAlarm(alarms: Alarm[], now: Date): Date | null {
	let best: Date | null = null;
	for (const a of alarms) {
		if (!a.enabled || !a.days.length) continue;
		for (let ahead = 0; ahead <= 7; ahead++) {
			const d = new Date(now.getFullYear(), now.getMonth(), now.getDate() + ahead, 0, 0, a.time);
			if (d > now && a.days.includes(d.getDay())) {
				if (!best || d < best) best = d;
				break;
			}
		}
	}
	return best;
}

// --- calls -----------------------------------------------------------------------------

export class RoutineApi {
	constructor(private authed: Authed) {}

	#call<T>(path: string, method = 'GET', body?: unknown): Promise<T> {
		return this.authed((token) =>
			callJson<T>(path, {
				method,
				headers: { ...bearer(token), ...(body === undefined ? {} : { 'Content-Type': 'application/json' }) },
				...(body === undefined ? {} : { body: JSON.stringify(body) }),
			}),
		);
	}

	async ratings(tracks: number[]): Promise<TrackRating[]> {
		const ids = [...new Set(tracks.filter((t) => trackId(t) !== null))].slice(0, 200);
		if (!ids.length) return [];
		const r = await this.#call<{ ratings?: unknown[] }>(`/v1/me/ratings?tracks=${ids.join(',')}`);
		return (r.ratings ?? []).map(sanitiseRating).filter((x): x is TrackRating => x !== null);
	}

	async rate(track: number, value: number): Promise<TrackRating | null> {
		if (trackId(track) === null) throw new RangeError('bad track');
		return sanitiseRating(await this.#call(`/v1/me/ratings/${track}`, 'PUT', { rating: ratingValue(value) }));
	}

	async style(): Promise<{ style: RatingStyle; count: number }> {
		const r = await this.#call<{ style?: unknown; count?: unknown }>('/v1/me/ratings/style');
		return { style: r.style === 'heart' ? 'heart' : 'stars', count: typeof r.count === 'number' ? r.count : 0 };
	}

	setStyle(style: RatingStyle): Promise<unknown> {
		return this.#call('/v1/me/ratings/style', 'PUT', { style: style === 'heart' ? 'heart' : 'stars' });
	}

	clearRatings(): Promise<{ removed: number }> {
		return this.#call('/v1/me/ratings', 'DELETE');
	}

	async alarms(p: string): Promise<AlarmListing> {
		return sanitiseAlarms(await this.#call(`/v1/players/${player(p)}/alarms`));
	}

	async addAlarm(p: string, c: AlarmChange): Promise<AlarmListing> {
		return sanitiseAlarms(await this.#call(`/v1/players/${player(p)}/alarms`, 'POST', alarmBody(c)));
	}

	async changeAlarm(p: string, id: string, c: AlarmChange): Promise<AlarmListing> {
		if (!ALARM_ID.test(id)) throw new RangeError('bad alarm id');
		return sanitiseAlarms(await this.#call(`/v1/players/${player(p)}/alarms/${id}`, 'PATCH', alarmBody(c)));
	}

	async deleteAlarm(p: string, id: string): Promise<AlarmListing> {
		if (!ALARM_ID.test(id)) throw new RangeError('bad alarm id');
		return sanitiseAlarms(await this.#call(`/v1/players/${player(p)}/alarms/${id}`, 'DELETE'));
	}

	async setAllAlarms(p: string, on: boolean): Promise<AlarmListing> {
		return sanitiseAlarms(await this.#call(`/v1/players/${player(p)}/alarms/enabled`, 'PUT', { enabled: !!on }));
	}
}
