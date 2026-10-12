// The sleep timer (docs/plans: Wave 1 §1), kept free of the DOM so it can be tested on its own.
//
// Every choice ends up as LMS's own `sleep <seconds>`, so the timer also works on hardware
// players and keeps going when the app is closed. "End of track/album/queue" is worked out
// from the queue (time left) and set the same way, then set again whenever that end moves
// (a seek, a skip, a pause, or a queue edit). At the end LMS fades the player's volume over
// the last minute (at most), then stops it and turns it off.
//
// The Android app has the same rules in Rust (mobile/src/sleep.rs); the tests share vectors.
import type { PlayerInfo, PlayerStatus, Track } from './types';

/** The fixed lengths offered, in minutes. */
export const SLEEP_MINUTES = [15, 30, 45, 60, 90] as const;
/** A custom length: 1 minute to a day. */
export const MIN_CUSTOM_MINUTES = 1;
export const MAX_CUSTOM_MINUTES = 24 * 60;

export type SleepEnd = 'track' | 'album' | 'queue';
export type SleepChoice = { kind: 'minutes'; minutes: number } | { kind: SleepEnd };
/** Which players the timer is for. */
export type SleepScope = 'player' | 'group' | 'all';

/** A whole number of minutes in the custom range, or null. */
export function customMinutes(v: unknown): number | null {
	const n = typeof v === 'number' ? v : typeof v === 'string' && /^\s*\d{1,4}\s*$/.test(v) ? Number(v) : NaN;
	if (!Number.isInteger(n) || n < MIN_CUSTOM_MINUTES || n > MAX_CUSTOM_MINUTES) return null;
	return n;
}

const finitePos = (v: unknown): number | null => {
	const n = typeof v === 'number' ? v : typeof v === 'string' && v.trim() !== '' ? Number(v) : NaN;
	return Number.isFinite(n) && n > 0 ? n : null;
};

/** The current track: the first entry of a `status - n` answer. */
export const currentTrack = (s: PlayerStatus | null | undefined): Track | undefined => s?.playlist_loop?.[0];

/**
 * Seconds until the end of the current track, the current album (the run of tracks from the
 * same album that follows it in the queue) or the queue. `upcoming` is the queue after the
 * current track, in playing order. Null when it can't be known: nothing playing, a stream
 * with no length, a track in range without a duration, or (for the queue) a queue longer
 * than what was read.
 */
export function remainingSeconds(end: SleepEnd, s: PlayerStatus | null | undefined, upcoming: readonly Track[] = []): number | null {
	const cur = currentTrack(s);
	if (!s || !cur) return null;
	const length = finitePos(s.duration) ?? finitePos(cur.duration);
	if (length === null) return null;
	const elapsed = Math.max(0, finitePos(s.time) ?? 0);
	let left = Math.max(0, length - elapsed);
	if (end === 'track') return Math.ceil(left);
	const album = finitePos(cur.album_id);
	if (end === 'album' && album === null) return Math.ceil(left);
	if (end === 'queue') {
		const index = Number(s.playlist_cur_index ?? 0);
		const total = Number(s.playlist_tracks ?? 0);
		const after = Number.isInteger(index) && Number.isInteger(total) ? Math.max(0, total - index - 1) : NaN;
		if (!(upcoming.length >= after)) return null; // only part of the queue was read
	}
	for (const t of upcoming) {
		if (end === 'album' && finitePos(t.album_id) !== album) break;
		const d = finitePos(t.duration);
		if (d === null) return null;
		left += d;
	}
	return Math.ceil(left);
}

/** Seconds for a choice (null when "end of …" can't be worked out). */
export function choiceSeconds(choice: SleepChoice, s: PlayerStatus | null | undefined, upcoming: readonly Track[] = []): number | null {
	if (choice.kind === 'minutes') {
		const m = customMinutes(choice.minutes);
		return m === null ? null : m * 60;
	}
	return remainingSeconds(choice.kind, s, upcoming);
}

/** A timer set to an end of the queue, kept in step with it. */
export interface Armed {
	end: SleepEnd;
	/** The players it was set on. */
	players: string[];
	/** When LMS should stop them (ms on the same clock as `now`). */
	endsAt: number;
	/** When it was last sent. */
	setAt: number;
}

/** Re-arm when the end has moved by more than this. */
export const REARM_TOLERANCE_S = 5;
/** LMS has had this long to report a timer just set before its absence means "cancelled". */
export const CONFIRM_MS = 10_000;

/**
 * The seconds to set the timer to again, if the end has moved (seek, skip, pause, queue
 * edit) by more than the tolerance; otherwise null. Nothing is known: leave it alone.
 */
export function rearmSeconds(armed: Armed, remaining: number | null, now: number, tolerance = REARM_TOLERANCE_S): number | null {
	if (remaining === null || !Number.isFinite(remaining)) return null;
	const wanted = now + remaining * 1000;
	return Math.abs(wanted - armed.endsAt) > tolerance * 1000 ? Math.max(1, Math.ceil(remaining)) : null;
}

/**
 * Whether an armed timer is over: LMS no longer reports one (it went off, or another
 * controller cancelled it) once it has had time to report it, or its end has passed.
 */
export function armedOver(armed: Armed, willSleepIn: unknown, now: number): boolean {
	const left = finitePos(willSleepIn);
	if (left !== null) return false;
	return now - armed.setAt > CONFIRM_MS || now > armed.endsAt + CONFIRM_MS;
}

/** Seconds left on the player's timer, as LMS reports it, or null when none is set. */
export function sleepLeft(s: PlayerStatus | null | undefined): number | null {
	const v = finitePos(s?.will_sleep_in);
	return v === null ? null : Math.ceil(v);
}

/** Players in the same sync group as `player` (itself included), from its status. */
export function groupOf(player: string, s: PlayerStatus | null | undefined): string[] {
	const ids = [s?.sync_master, ...String(s?.sync_slaves ?? '').split(',')]
		.map((x) => String(x ?? '').trim().toLowerCase())
		.filter((x) => /^[0-9a-f]{2}(:[0-9a-f]{2}){5}$/.test(x));
	const out = [player.toLowerCase(), ...ids];
	return [...new Set(out)];
}

/**
 * The players a timer is set on: the selected one, its sync group, or every player listed
 * (the gateway lists only players this user may control) that is connected.
 */
export function sleepTargets(scope: SleepScope, player: string, s: PlayerStatus | null | undefined, players: readonly PlayerInfo[]): string[] {
	if (!player) return [];
	if (scope === 'group') return groupOf(player, s);
	if (scope === 'all') {
		const all = players.filter((p) => p.connected !== 0).map((p) => p.playerid.toLowerCase());
		return [...new Set([player.toLowerCase(), ...all])];
	}
	return [player.toLowerCase()];
}

// --- fade-out ---------------------------------------------------------------------------

/** Fade-out lengths offered in Settings (seconds; 0 is off). */
export const FADE_CHOICES = [0, 10, 30, 60, 90, 120] as const;
export const DEFAULT_FADE_S = 30;
export const MAX_FADE_S = 120;

/** A saved fade length, checked: whole seconds 0–120, else the default. */
export function sanitiseFade(v: unknown): number {
	const n = typeof v === 'number' ? v : typeof v === 'string' && /^\d{1,3}$/.test(v) ? Number(v) : NaN;
	return Number.isInteger(n) && n >= 0 && n <= MAX_FADE_S ? n : DEFAULT_FADE_S;
}

const fadeKey = (user: string) => `anywhere.sleep.fade.${user.replace(/[^A-Za-z0-9._@-]/g, '_').slice(0, 128)}`;

type Store = Pick<Storage, 'getItem' | 'setItem'>;
function safeLocal(): Store | undefined {
	try {
		return globalThis.localStorage;
	} catch {
		return undefined;
	}
}

/** The user's fade-out length (kept per user, in this browser for now). */
export function loadFade(user: string, store: Store | undefined = safeLocal()): number {
	try {
		return sanitiseFade(store?.getItem(fadeKey(user)) ?? undefined);
	} catch {
		return DEFAULT_FADE_S;
	}
}

export function saveFade(user: string, seconds: number, store: Store | undefined = safeLocal()) {
	try {
		store?.setItem(fadeKey(user), String(sanitiseFade(seconds)));
	} catch {
		/* not essential */
	}
}

/**
 * This device's own output gain while a timer runs: full until `fade` seconds are left, then
 * down to silence at the end (an ease-in curve, so the drop sounds even). 1 when no timer.
 */
export function fadeGain(secondsLeft: number | null, fade: number): number {
	if (secondsLeft === null || !Number.isFinite(secondsLeft) || fade <= 0) return 1;
	if (secondsLeft <= 0) return 0;
	if (secondsLeft >= fade) return 1;
	const x = secondsLeft / fade;
	return x * x;
}

// --- words ------------------------------------------------------------------------------

/** "45 s", "27 min", "1 h 5 min". */
export function formatLeft(seconds: number): string {
	const s = Math.max(0, Math.ceil(seconds));
	if (s < 60) return `${s} s`;
	const m = Math.ceil(s / 60);
	if (m < 60) return `${m} min`;
	const h = Math.floor(m / 60);
	const r = m % 60;
	return r ? `${h} h ${r} min` : `${h} h`;
}

export const END_LABELS: Record<SleepEnd, string> = {
	track: 'End of this track',
	album: 'End of this album',
	queue: 'End of the queue',
};
