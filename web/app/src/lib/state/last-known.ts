// What the selected player was last seen doing, kept on this device so that a page loaded
// while the music server is busy or out of reach shows it (with the connection message)
// rather than "Nothing to control yet". Per user, forgotten at sign-out; nothing secret.
// Storage is untrusted: anything that doesn't look right is ignored.
import type { PlayerInfo, PlayerStatus } from '#lib/lms/types';

export const LAST_KNOWN_KEY = 'anywhere.lastKnown';
/** Most bytes kept (a status with its current track is a few hundred). */
export const LAST_KNOWN_MAX = 32 * 1024;

export interface LastKnown {
	user: string;
	player: PlayerInfo;
	now: PlayerStatus;
}

type Get = Pick<Storage, 'getItem'>;
type Set = Pick<Storage, 'setItem' | 'removeItem'>;

function local(): Storage | undefined {
	try {
		return globalThis.localStorage;
	} catch {
		return undefined;
	}
}

const isObject = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object' && !Array.isArray(v);

/** Keep `v` (or forget it, if it is too big to keep). */
export function saveLastKnown(v: LastKnown, storage: Set | undefined = local()) {
	try {
		const raw = JSON.stringify({ user: v.user, player: v.player, now: v.now });
		if (raw.length > LAST_KNOWN_MAX) storage?.removeItem(LAST_KNOWN_KEY);
		else storage?.setItem(LAST_KNOWN_KEY, raw);
	} catch {
		/* storage blocked or full: not essential */
	}
}

/** What `user`'s player `playerId` was last seen doing, if this device knows. */
export function loadLastKnown(user: string, playerId: string, storage: Get | undefined = local()): LastKnown | null {
	if (!user || !playerId) return null;
	let v: unknown;
	try {
		const raw = storage?.getItem(LAST_KNOWN_KEY);
		if (!raw || raw.length > LAST_KNOWN_MAX) return null;
		v = JSON.parse(raw);
	} catch {
		return null;
	}
	if (!isObject(v) || v.user !== user || !isObject(v.player) || !isObject(v.now)) return null;
	const p = v.player;
	if (p.playerid !== playerId || typeof p.name !== 'string') return null;
	const now = v.now;
	if (now.playlist_loop !== undefined && !(Array.isArray(now.playlist_loop) && now.playlist_loop.every(isObject))) return null;
	return { user, player: p as unknown as PlayerInfo, now: now as PlayerStatus };
}

export function forgetLastKnown(storage: Pick<Storage, 'removeItem'> | undefined = local()) {
	try {
		storage?.removeItem(LAST_KNOWN_KEY);
	} catch {
		/* best effort */
	}
}
