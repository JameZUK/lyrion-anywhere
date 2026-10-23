// "Continue on <player>" (SPEC §8.4): back online, the queue the offline player was playing
// from this device's downloads can be handed to an LMS player, never by itself: the app
// offers it (see ContinueOn.svelte) and the user picks the player or says "Not now".
//
// The hand-off:
// 1. Each queued download is checked with GET /v1/download/track/{id}/info (the id it was
//    downloaded with). The gateway answers with the stable key of the file that id names
//    now; the id is used only if that is the download's own key. A track LMS no longer has
//    (404), or whose id now names another file (a rescan renumbered it), is skipped and
//    counted. Any other failure (signed out, downloads not allowed, no connection) stops the
//    hand-off before anything is sent to a player.
// 2. The confirmed ids are loaded on the player in queue order through the gateway's RPC
//    policy, as Transfer's copy recipe does (docs/PROTOCOL.md): power on, shuffle off, the
//    id lists (`load`, then `add`), a jump to the current track, a seek to the position, and
//    a pause if the downloads were paused.
// 3. Only then is the offline player stopped. If anything fails on the way it is left alone,
//    playing (or paused) as it was.
import * as cmd from '#lib/lms/commands';
import type { Command } from '#lib/api/rpc';
import type { StoredTrack } from './types';

/** Most tracks handed over: a longer queue keeps this many from around the current track. */
export const MAX_HANDOFF = 1000;
/** Lookups in flight at once. */
export const PARALLEL = 4;

/** The offline queue at one moment, as handed over. */
export interface QueueSnapshot {
	/** Whose downloads they are: only that user may continue them. */
	user: string;
	tracks: { id: number; key: string }[];
	index: number;
	/** Seconds into the current track. */
	position: number;
	playing: boolean;
}

/** What the offline player looks like to the hand-off. */
export interface QueueSource {
	readonly queue: readonly Pick<StoredTrack, 'id' | 'key'>[];
	readonly index: number;
	readonly position: number;
	readonly playing: boolean;
	readonly user: string;
}

/** The player's queue now, or null with nothing queued. */
export function snapshotOf(p: QueueSource): QueueSnapshot | null {
	if (!p.queue.length || !p.user || p.index < 0 || p.index >= p.queue.length) return null;
	return {
		user: p.user,
		tracks: p.queue.map((t) => ({ id: t.id, key: t.key })),
		index: p.index,
		position: Number.isFinite(p.position) && p.position > 0 ? p.position : 0,
		playing: p.playing,
	};
}

const sameTracks = (a: QueueSnapshot, b: QueueSnapshot) => a.tracks.length === b.tracks.length && a.tracks.every((t, i) => t.key === b.tracks[i]!.key && t.id === b.tracks[i]!.id);

/** `/info` for an id: what the server says the id names now, or null when it doesn't know it (404). */
export type Lookup = (id: number) => Promise<{ id: number; key: string } | null>;

const KEY = /^[0-9a-f]{64}$/;
const plausible = (id: number) => Number.isSafeInteger(id) && id > 0 && id < 1e12;

/**
 * Each track's current LMS id, in order (null: no longer in the library as that file).
 * Each id is asked about once (`cache` keeps answers across calls). Throws on any failure
 * other than an unknown id, so nothing half-checked is ever sent.
 */
export async function resolveTracks(tracks: readonly { id: number; key: string }[], lookup: Lookup, cache = new Map<number, string | null>(), parallel = PARALLEL): Promise<(number | null)[]> {
	const todo = [...new Set(tracks.filter((t) => plausible(t.id) && KEY.test(t.key)).map((t) => t.id))].filter((id) => !cache.has(id));
	let next = 0;
	const worker = async () => {
		while (next < todo.length) {
			const id = todo[next++]!;
			const r = await lookup(id);
			cache.set(id, r && r.id === id && KEY.test(r.key) ? r.key : null);
		}
	};
	await Promise.all(Array.from({ length: Math.min(parallel, todo.length) }, worker));
	return tracks.map((t) => (plausible(t.id) && KEY.test(t.key) && cache.get(t.id) === t.key ? t.id : null));
}

export interface HandoffPlan {
	ids: number[];
	/** Index into `ids` to play from. */
	start: number;
	/** Seconds into that track (0 when the current track itself was skipped). */
	seek: number;
	paused: boolean;
	/** Tracks that couldn't be handed over. */
	skipped: number;
}

/** What to load, from the snapshot and each track's resolved id; null when nothing resolved. */
export function planHandoff(s: QueueSnapshot, resolved: readonly (number | null)[]): HandoffPlan | null {
	// A very long queue: keep the stretch from the current track on (and before it if room).
	const from = s.tracks.length > MAX_HANDOFF ? Math.max(0, Math.min(s.index, s.tracks.length - MAX_HANDOFF)) : 0;
	const to = Math.min(s.tracks.length, from + MAX_HANDOFF);
	const ids: number[] = [];
	let start = -1;
	let exact = false;
	for (let i = from; i < to; i++) {
		const id = resolved[i];
		if (id === null || id === undefined) continue;
		if (i === s.index) {
			start = ids.length;
			exact = true;
		} else if (i > s.index && start < 0) start = ids.length;
		ids.push(id);
	}
	if (!ids.length) return null;
	// The current track and everything after it skipped: from the last one before it.
	if (start < 0) start = ids.length - 1;
	return { ids, start, seek: exact ? s.position : 0, paused: !s.playing, skipped: s.tracks.length - ids.length };
}

/** The commands for the target player, in order (every id checked; each list fits the gateway's limit). */
export function handoffCommands(p: HandoffPlan, seekNow = p.seek): Command[] {
	const chunks = cmd.chunkIds(p.ids);
	const first = chunks[0]!;
	const out: Command[] = [
		cmd.power(true),
		// The list goes in play order, so the player must not shuffle it again.
		cmd.shuffle(0),
		cmd.loadTracks(first, 'load', p.start < first.length ? p.start : 0),
		...chunks.slice(1).map((c) => cmd.loadTracks(c, 'add')),
	];
	if (p.start >= first.length) out.push(cmd.jump(p.start));
	// A second or two in is as good as the start.
	if (seekNow >= 2) out.push(cmd.seek(Math.floor(seekNow)));
	if (p.paused) out.push(cmd.pause(true));
	return out;
}

export type HandoffResult = { ok: true; moved: number; skipped: number } | { ok: false; reason: 'empty' | 'none' | 'changed' | 'error'; message?: string };

export interface HandoffDeps {
	/** The queue now (null: nothing queued). Read again before loading, as it may change. */
	current: () => QueueSnapshot | null;
	lookup: Lookup;
	rpc: (player: string, c: Command) => Promise<unknown>;
	/** Stop the offline player (called only once the player has the queue). */
	stopLocal: () => void;
	now?: () => number;
}

/** Hand the offline queue to `target` (a player this user may control: the gateway refuses others). */
export async function continueOn(target: string, d: HandoffDeps): Promise<HandoffResult> {
	let snap = d.current();
	if (!snap) return { ok: false, reason: 'empty' };
	const cache = new Map<number, string | null>();
	let resolved: (number | null)[] = [];
	try {
		cmd.playerId(target);
		// The queue may change while the tracks are checked: check the new one (answers kept).
		for (let tries = 0; ; tries++) {
			resolved = await resolveTracks(snap.tracks, d.lookup, cache);
			const fresh = d.current();
			if (!fresh) return { ok: false, reason: 'empty' };
			if (fresh.user !== snap.user) return { ok: false, reason: 'changed' };
			const same = sameTracks(fresh, snap);
			snap = fresh;
			if (same) break;
			if (tries >= 2) return { ok: false, reason: 'changed' };
		}
	} catch (e) {
		return { ok: false, reason: 'error', message: (e as Error).message };
	}
	const plan = planHandoff(snap, resolved);
	if (!plan) return { ok: false, reason: 'none' };
	const now = d.now ?? (() => Date.now());
	const readAt = now();
	try {
		for (const c of handoffCommands(plan)) {
			// Playing: seek to where the downloads have got to by now, not where they were.
			const step = c[0] === 'time' && !plan.paused ? cmd.seek(Math.floor(plan.seek + (now() - readAt) / 1000)) : c;
			await d.rpc(target, step);
		}
	} catch (e) {
		return { ok: false, reason: 'error', message: (e as Error).message };
	}
	d.stopLocal();
	return { ok: true, moved: plan.ids.length, skipped: plan.skipped };
}

/** What to tell the user afterwards. `here`: the downloads are still playing (or paused) on this device. */
export function handoffText(r: HandoffResult, target: string, here = true): string {
	if (r.ok) {
		const skipped = r.skipped
			? ` ${r.skipped} ${r.skipped === 1 ? 'track was' : 'tracks were'} skipped: ${r.skipped === 1 ? 'it isn’t' : 'they aren’t'} in the library any more.`
			: '';
		return `Now playing on ${target}.${skipped}`;
	}
	const still = here ? ' Your downloads carry on here.' : '';
	switch (r.reason) {
		case 'empty':
			return 'Nothing to continue: the downloads queue is empty.';
		case 'none':
			return `None of these tracks are in the library any more, so nothing was sent to ${target}.${still}`;
		case 'changed':
			return `The downloads queue kept changing, so nothing was sent to ${target}. Try again.`;
		case 'error':
			return `Couldn’t continue on ${target}${r.message ? ` (${r.message})` : ''}.${still}`;
	}
}

/** A player the queue can go to. */
export interface HandoffChoice {
	id: string;
	name: string;
	/** This device's own player. */
	device: boolean;
}

/**
 * Where the queue can go: this device's own player (when this browser can play), then the
 * players this user may control, as LMS lists them for this user (the gateway filters them).
 */
export function handoffChoices(players: readonly { playerid: string; name?: unknown }[], device: { mac: string; supported: boolean }): HandoffChoice[] {
	const out: HandoffChoice[] = [];
	if (device.supported && device.mac) out.push({ id: device.mac, name: 'This device', device: true });
	for (const p of players) {
		if (p.playerid === device.mac || out.some((c) => c.id === p.playerid)) continue;
		out.push({ id: p.playerid, name: typeof p.name === 'string' && p.name.trim() ? p.name : p.playerid, device: false });
	}
	return out;
}

/** The player offered first: the one last in use (this device, when it was), else this device, else the first. */
export function defaultChoice(choices: readonly HandoffChoice[], inUse: string): string {
	if (inUse && choices.some((c) => c.id === inUse)) return inUse;
	return (choices.find((c) => c.device) ?? choices[0])?.id ?? '';
}

/**
 * When to offer the hand-off. The app was offline (the server out of reach, or the browser
 * offline) and is now signed in and online again, with a queue of downloads: offer it once.
 * A queue stopped by an expired sign-in is remembered (in memory) and offered once the same
 * user has signed in again; never to anybody else.
 */
export class HandoffOffer {
	/** The user it is offered to (null: not offered). */
	offered = $state<string | null>(null);
	/** Running: the player's name. */
	busy = $state('');
	#wasOffline = false;
	#kept: QueueSnapshot | null = null;

	/** The server can't be reached, or the browser has no connection. */
	wentOffline() {
		this.#wasOffline = true;
	}

	/** The offline player is about to be stopped by a sign-out that wasn't asked for: keep its queue. */
	keep(s: QueueSnapshot | null) {
		if (s) this.#kept = { ...s, playing: false };
	}

	/** The remembered queue for `user` (not for anyone else). */
	kept(user: string): QueueSnapshot | null {
		return this.#kept && this.#kept.user === user ? this.#kept : null;
	}

	/** Signed in as `user` and online again; `live` is the offline player's queue now. */
	backOnline(user: string, live: QueueSnapshot | null) {
		if (!this.#wasOffline) return;
		this.#wasOffline = false;
		if (this.#kept && this.#kept.user !== user) this.#kept = null;
		const mine = live && live.user === user ? live : this.kept(user);
		this.offered = mine ? user : null;
	}

	/** The queue to hand over now (the live one first), or null: then the offer goes. */
	queue(live: QueueSnapshot | null): QueueSnapshot | null {
		const user = this.offered;
		if (!user) return null;
		if (live && live.user === user) return live;
		return this.kept(user);
	}

	/** "Not now", or done. */
	dismiss() {
		this.offered = null;
		this.#kept = null;
	}

	/** Signed out on purpose: forget everything. */
	reset() {
		this.offered = null;
		this.busy = '';
		this.#kept = null;
		this.#wasOffline = false;
	}
}
