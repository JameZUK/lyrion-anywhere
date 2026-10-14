// What still works while the server is out of reach, and what to tell it when it is back.
// Pure decisions (no state, no I/O), shared by every control surface: the player bar, Now
// playing, the queue views and the lock screen. See SPEC.md, "Controls while the server is
// out of reach".
import type { Command } from '$lib/api/rpc';
import type { Reachability } from '$lib/state/reachability';
import { trackId } from './resume';

/** Where a queue entry's audio is on this device, if anywhere. */
export type LocalKind = 'downloaded' | 'device';

export const MARK_LABELS: Record<LocalKind, string> = {
	downloaded: 'Downloaded',
	device: 'On this device',
};

/** Within this many seconds of a track's start, Previous goes to the track before. */
export const RESTART_AFTER_S = 3;

export interface MarkInput {
	/** The entry's LMS track id. */
	id: unknown;
	/** Its place in the queue. */
	index: number;
	/** Is that track downloaded to this device? */
	downloaded: (id: unknown) => boolean;
	/** This device is the player being controlled. */
	thisDevice: boolean;
	/** The queue entry whose whole stream this device's player holds in memory (-1: none). */
	heldIndex: number;
}

/**
 * The queue mark for an entry: "Downloaded" for any player (the download is this device's
 * own), "On this device" only while this device is the player and holds that track's
 * stream.
 */
export function queueMark(m: MarkInput): LocalKind | null {
	if (m.downloaded(m.id)) return 'downloaded';
	if (m.thisDevice && m.heldIndex >= 0 && m.index === m.heldIndex) return 'device';
	return null;
}

export interface QueueState {
	/** The entry playing (or paused on). */
	index: number;
	/** Entries in the queue. */
	count: number;
	/** LMS's repeat: 0 off, 1 this track, 2 the whole queue. */
	repeat: number;
}

/** The entry after `index` if it can be played here (repeat-all wraps), else null. */
export function nextIndex(q: QueueState, available: (i: number) => boolean): number | null {
	if (q.count <= 0 || q.index < 0) return null;
	let i = q.index + 1;
	if (i >= q.count) {
		if (q.repeat !== 2) return null;
		i = 0;
	}
	return i !== q.index && available(i) ? i : null;
}

export type Previous = { restart: true } | { index: number };

/**
 * What Previous does here, as LMS would: more than a few seconds in, back to the start of
 * the track (only if this device has the track from its start); otherwise the entry before
 * (repeat-all wraps), if this device can play it. Null: nothing it can do here.
 */
export function previousTarget(q: QueueState, elapsed: number, available: (i: number) => boolean, fromStart: (i: number) => boolean): Previous | null {
	if (q.count <= 0 || q.index < 0) return null;
	if (elapsed > RESTART_AFTER_S && fromStart(q.index)) return { restart: true };
	let i = q.index - 1;
	if (i < 0) {
		if (q.repeat !== 2) return null;
		i = q.count - 1;
	}
	return i !== q.index && available(i) ? { index: i } : null;
}

/**
 * When a track played here ends: repeat it (repeat one, if it can be played from its
 * start), else go on to the next entry if it is here. Null: stop and wait for the server.
 */
export function afterEnd(q: QueueState, available: (i: number) => boolean, fromStart: (i: number) => boolean): number | null {
	if (q.repeat === 1 && q.index >= 0 && fromStart(q.index)) return q.index;
	return nextIndex(q, available);
}

/** Why playback stopped at the end of a track, when the next one isn't here. */
export const NEXT_NEEDS_SERVER = 'Next track needs your server.';

/** Which controls work now. `jump` says whether a queue entry can be played. */
export interface Gates {
	playPause: boolean;
	next: boolean;
	previous: boolean;
	seek: boolean;
	volume: boolean;
	/** Shuffle, repeat, queue edits, the sleep timer, power, sync, library play actions. */
	other: boolean;
	jump: (i: number) => boolean;
}

export interface GateInput {
	reach: Reachability;
	/** There is a player status to act on. */
	hasNow: boolean;
	/** This device is the player being controlled. */
	thisDevice: boolean;
	queue: QueueState;
	elapsed: number;
	/** Can entry `i` be played here start to finish (a download, or the stream it holds)? */
	available: (i: number) => boolean;
	/** Can entry `i` be played here from its very start (a download)? */
	fromStart: (i: number) => boolean;
	/** What plays here now: this device's stream, a download, or nothing yet. */
	source: 'stream' | 'download' | null;
	/** The stream can be paused or resumed here (it is playing, or paused here). */
	streamControllable: boolean;
}

const NONE: Gates = { playPause: false, next: false, previous: false, seek: false, volume: false, other: false, jump: () => false };

export function gates(g: GateInput): Gates {
	if (g.reach === 'online') {
		const on = g.hasNow;
		// (A queue row can always be asked for: LMS says if there is nothing to play.)
		return { playPause: on, next: on, previous: on, seek: on, volume: on, other: on, jump: () => true };
	}
	// Out of reach (or busy): a remote player can't be told anything.
	if (!g.thisDevice || !g.hasNow) return NONE;
	const q = g.queue;
	const playPause = g.source === 'download' || (g.source === 'stream' ? g.streamControllable : g.fromStart(q.index));
	return {
		playPause,
		next: nextIndex(q, g.available) !== null,
		previous: previousTarget(q, g.elapsed, g.available, g.fromStart) !== null,
		// Within the audio held here: anywhere in a download; a stream can't be sought without LMS.
		seek: g.source === 'download',
		// This device's volume is LMS's: it can't be changed without the server.
		volume: false,
		other: false,
		jump: (i) => (i === q.index ? playPause : i >= 0 && i < q.count && g.available(i)),
	};
}

/** What this device did while the server was out of reach. */
export interface LocalOutcome {
	/** The entry it ended on, and the LMS track id there (as it was before the outage). */
	index: number;
	track: unknown;
	/** Seconds into it. */
	time: number;
	paused: boolean;
	/** It is still playing the stream LMS sent, on the same connection (nothing to restart). */
	sameStream: boolean;
}

/** What LMS says now (`status`). */
export interface LmsNow {
	mode?: string;
	playlist_cur_index?: number | string;
	playlist_tracks?: number;
}

export type Reconcile =
	/** LMS's queue changed meanwhile: drop what was done here and follow LMS. */
	| { follow: true; commands: [] }
	/** Tell LMS, then let this device's output follow it once `release` is seen. */
	| { follow: false; commands: Command[]; release: 'now' | 'flush' | 'paused' };

/**
 * Back in reach after acting here: this device's state is the truth, but only if LMS still
 * has the same track at that place in the queue (someone else may have changed it). Built
 * on the reload resume (resume.ts): the same index, id and time checks.
 */
export function reconcile(local: LocalOutcome, now: LmsNow | null, trackAtIndex: unknown): Reconcile {
	const tracks = typeof now?.playlist_tracks === 'number' ? now.playlist_tracks : 0;
	const want = trackId(local.track);
	if (!now || local.index < 0 || local.index >= tracks || want === undefined || String(trackId(trackAtIndex)) !== String(want)) {
		return { follow: true, commands: [] };
	}
	const lmsIndex = Number(now.playlist_cur_index);
	const lmsPaused = now.mode === 'pause';
	if (local.sameStream && lmsIndex === local.index && now.mode !== 'stop') {
		// LMS's position is this player's own: only the pause state can differ.
		if (local.paused === lmsPaused) return { follow: false, commands: [], release: 'now' };
		return { follow: false, commands: [['pause', local.paused ? 1 : 0]], release: local.paused ? 'paused' : 'now' };
	}
	const commands: Command[] = [['playlist', 'index', local.index]];
	if (local.time >= 2) commands.push(['time', Math.floor(local.time)]);
	if (local.paused) commands.push(['pause', 1]);
	return { follow: false, commands, release: local.paused ? 'paused' : 'flush' };
}
