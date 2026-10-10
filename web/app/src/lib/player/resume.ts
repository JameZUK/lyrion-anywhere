// Resume after a reload: where this device was in the queue, so playback can carry on.
//
// When the page goes away while this device is the selected, playing player, the app notes
// the queue position (track index, track id and time). After the reload the device starts
// again by itself; if LMS hasn't carried on playing by then, the app jumps back to that
// point, provided the same track is still at that place in the queue.
import type { Command } from '$lib/api/rpc';
import * as cmd from '$lib/lms/commands';

export interface ResumePoint {
	mac: string;
	playing: boolean;
	index: number;
	time: number;
	/** When it was saved (ms since the epoch). */
	at: number;
	/** LMS's id of the track that was playing, if known. */
	track?: number | string;
}

type Store = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;
const KEY = 'anywhere.device.resume';
/** Older than this, a resume point is stale: the user has moved on. */
export const MAX_AGE_MS = 10 * 60_000;
const MAC = /^[0-9a-f]{2}(:[0-9a-f]{2}){5}$/;

function safeSession(): Store | undefined {
	try {
		return globalThis.sessionStorage;
	} catch {
		return undefined;
	}
}

export function saveResume(p: ResumePoint, store: Store | undefined = safeSession()) {
	try {
		store?.setItem(KEY, JSON.stringify(p));
	} catch {
		/* not essential */
	}
}

export function clearResume(store: Store | undefined = safeSession()) {
	try {
		store?.removeItem(KEY);
	} catch {
		/* not essential */
	}
}

const count = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) && v >= 0 ? v : null);

/** A track id as LMS gives it: an integer (negative for remote tracks), or a short string. */
export function trackId(v: unknown): number | string | undefined {
	if (typeof v === 'number' && Number.isSafeInteger(v)) return v;
	if (typeof v === 'string' && v.length > 0 && v.length <= 128 && /^[\w:.-]+$/.test(v)) return v;
	return undefined;
}

/** The saved point, if it is well formed and recent. */
export function loadResume(store: Store | undefined = safeSession(), now = Date.now()): ResumePoint | null {
	try {
		const v = JSON.parse(store?.getItem(KEY) ?? 'null') as Partial<ResumePoint> | null;
		if (!v || typeof v !== 'object') return null;
		const index = count(v.index);
		const time = count(v.time);
		const at = count(v.at);
		if (typeof v.mac !== 'string' || !MAC.test(v.mac) || index === null || time === null || at === null) return null;
		if (now - at > MAX_AGE_MS || at > now + 60_000) return null;
		const p: ResumePoint = { mac: v.mac, playing: v.playing === true, index: Math.floor(index), time, at };
		if (v.track !== undefined) {
			const track = trackId(v.track);
			if (track === undefined) return null;
			p.track = track;
		}
		return p;
	} catch {
		return null;
	}
}

/** What LMS says about the player now (from `status`). */
export interface NowStatus {
	mode?: string;
	playlist_cur_index?: number | string;
	'playlist_tracks'?: number;
}

/**
 * Commands that put playback back where it was, or none if there is nothing to do: it wasn't
 * playing, LMS already carries on, or the queue no longer has that track. `trackAtIndex` is
 * the id of the track now at the saved index: if the point knows its track and that differs,
 * the queue has changed, and jumping would play something else.
 */
export function resumeCommands(p: ResumePoint, now: NowStatus | null, trackAtIndex?: unknown): Command[] {
	if (!p.playing || !now || now.mode === 'play') return [];
	const tracks = typeof now.playlist_tracks === 'number' ? now.playlist_tracks : 0;
	if (p.index >= tracks) return [];
	if (p.track !== undefined && String(trackId(trackAtIndex)) !== String(p.track)) return [];
	const out: Command[] = [['playlist', 'index', p.index]];
	// Seek only if it's worth it (a second or two in is as good as the start).
	if (p.time >= 2) out.push(['time', Math.floor(p.time)]);
	return out;
}

type Rpc = <T = Record<string, unknown>>(player: string, cmd: Command) => Promise<T>;

/**
 * Ask LMS about this device's own player (not whichever player the app has selected), and
 * work out the commands to resume it.
 */
export async function planResume(p: ResumePoint, rpc: Rpc): Promise<Command[]> {
	const now = await rpc<NowStatus>(p.mac, cmd.status());
	if (!resumeCommands(p, now, p.track).length) return []; // nothing to do, whatever is queued
	if (p.track === undefined) return resumeCommands(p, now);
	const q = await rpc<{ playlist_loop?: { id?: unknown }[] }>(p.mac, cmd.queue(p.index, 1));
	return resumeCommands(p, now, q.playlist_loop?.[0]?.id);
}
