// Endless stations (docs/plans/wave1-design.md §6): start one on a player, then keep its
// queue topped up while it plays. The same rules as the Android app (mobile/src).
//
// - Built-in stations come back as track ids: the first batch replaces the queue
//   (`playlistcontrol cmd:load`), later ones are added (`cmd:add`), in lists that fit the
//   gateway's limit.
// - A single-genre station is LMS's own Random Mix: its command is sent once and LMS keeps
//   the queue filled itself.
// - A station pinned from a menu is run by the gateway (it re-checks the menu first); it
//   is refilled the same way only if the menu item offers "add".
// - Refill when fewer than LOW_WATER tracks are left after the current one, never two at
//   once, leaving out what is already queued (the gateway also avoids the last 200 plays).
// - The station stops when its player changes, or the user plays something else (the
//   current track is none of the station's).
import type { Command } from '#lib/api/rpc';
import type { NextBatch, NextRequest } from '#lib/api/me';
import { chunkIds, loadTracks } from '#lib/lms/commands';

/** Refill when fewer than this many tracks are left after the current one. */
export const LOW_WATER = 5;
/** Tracks asked for each time. */
export const BATCH = 20;
/** Queued track ids sent as "leave these out", at most. */
export const MAX_EXCLUDE = 200;

/** What a player's status says about its queue (LMS `status`). */
export interface QueueStatus {
	playlist_tracks?: number | string;
	playlist_cur_index?: number | string;
	playlist_loop?: { id?: unknown }[];
}

const num = (v: unknown): number | null => {
	const n = typeof v === 'number' ? v : typeof v === 'string' && /^\d{1,9}$/.test(v) ? Number(v) : NaN;
	return Number.isFinite(n) && n >= 0 ? n : null;
};

/** Tracks left after the current one, or null if the status doesn't say. */
export function remaining(s: QueueStatus | null | undefined): number | null {
	const total = num(s?.playlist_tracks);
	if (total === null) return null;
	if (total === 0) return 0;
	const cur = num(s?.playlist_cur_index) ?? 0;
	return Math.max(0, total - cur - 1);
}

/** Is the queue low enough to add the next batch? */
export function needsRefill(s: QueueStatus | null | undefined, threshold = LOW_WATER): boolean {
	const left = remaining(s);
	return left !== null && left < threshold;
}

/** The current track's id, if it is a library track. */
export function currentTrack(s: QueueStatus | null | undefined): number | null {
	const v = s?.playlist_loop?.[0]?.id;
	return typeof v === 'number' && Number.isSafeInteger(v) && v > 0 ? v : null;
}

/** Commands that put these tracks in the queue: replace it (`load`) or append (`add`). */
export function batchCommands(ids: readonly number[], how: 'load' | 'add'): Command[] {
	const clean = ids.filter((x) => Number.isSafeInteger(x) && x > 0);
	return chunkIds(clean).map((part, i) => loadTracks(part, i === 0 ? how : 'add'));
}

/** Library track ids in a queue listing, at most MAX_EXCLUDE, without repeats. */
export function queuedIds(loop: readonly { id?: unknown }[] | undefined): number[] {
	const out = new Set<number>();
	for (const t of loop ?? []) {
		if (typeof t.id === 'number' && Number.isSafeInteger(t.id) && t.id > 0) out.add(t.id);
		if (out.size >= MAX_EXCLUDE) break;
	}
	return [...out];
}

export interface RunnerDeps {
	next(station: number, req: NextRequest): Promise<NextBatch>;
	rpc(player: string, cmd: Command): Promise<unknown>;
	/** Track ids in the player's queue now (to leave out). */
	queue(player: string): Promise<number[]>;
}

export type RunnerState = 'idle' | 'starting' | 'playing' | 'stopped';

/** One station playing on one player. */
export class StationRunner {
	station: number | null = null;
	player = '';
	state: RunnerState = 'idle';
	/** Keep adding batches? */
	refill = false;
	error = '';
	/** Track ids this station put in the queue (to tell when the user played something else). */
	#mine = new Set<number>();
	/** Whether we know the station's track ids (menu stations: we don't). */
	#knowsTracks = false;
	#busy = false;
	#run = 0;

	constructor(private deps: RunnerDeps) {}

	get active(): boolean {
		return this.state === 'starting' || this.state === 'playing';
	}

	/** Start a station on a player: replaces the queue. Throws on failure. */
	async start(station: number, player: string): Promise<NextBatch> {
		const run = ++this.#run;
		this.station = station;
		this.player = player;
		this.state = 'starting';
		this.refill = false;
		this.error = '';
		this.#mine.clear();
		this.#knowsTracks = false;
		this.#busy = true;
		try {
			const batch = await this.deps.next(station, { player, start: true, count: BATCH });
			if (run !== this.#run) return batch;
			await this.#apply(batch, 'load');
			if (run !== this.#run) return batch;
			this.state = batch.mode === 'none' || batch.mode === 'randomplay' ? 'stopped' : 'playing';
			if (batch.mode === 'tracks' && batch.tracks.length === 0) {
				this.state = 'stopped';
				this.error = 'This station has nothing new to play right now.';
			}
			return batch;
		} catch (e) {
			if (run === this.#run) {
				this.state = 'stopped';
				this.error = (e as Error).message;
			}
			throw e;
		} finally {
			if (run === this.#run) this.#busy = false;
		}
	}

	stop() {
		this.#run++;
		this.state = 'stopped';
		this.refill = false;
		this.#busy = false;
	}

	/**
	 * The player's status changed: stop if the user moved on, refill if the queue is low.
	 * Returns the refill under way, if one was started.
	 */
	onStatus(player: string, s: QueueStatus | null | undefined): Promise<void> | null {
		if (this.state !== 'playing') return null;
		if (player !== this.player) {
			this.stop();
			return null;
		}
		const cur = currentTrack(s);
		if (this.#knowsTracks && cur !== null && this.#mine.size && !this.#mine.has(cur)) {
			this.stop(); // something else is playing now
			return null;
		}
		if (!this.refill || this.#busy || !needsRefill(s)) return null;
		return this.#refill();
	}

	async #refill(): Promise<void> {
		const run = this.#run;
		const station = this.station;
		if (station === null) return;
		this.#busy = true;
		try {
			const exclude = await this.deps.queue(this.player).catch(() => [] as number[]);
			if (run !== this.#run) return;
			const batch = await this.deps.next(station, { player: this.player, start: false, count: BATCH, exclude });
			if (run !== this.#run) return;
			await this.#apply(batch, 'add');
			if (batch.mode === 'tracks' && batch.tracks.length === 0) this.refill = false;
		} catch (e) {
			if (run === this.#run) {
				this.error = (e as Error).message;
				this.stop();
			}
		} finally {
			if (run === this.#run) this.#busy = false;
		}
	}

	async #apply(batch: NextBatch, how: 'load' | 'add') {
		switch (batch.mode) {
			case 'tracks':
				this.#knowsTracks = true;
				for (const id of batch.tracks) this.#mine.add(id);
				for (const c of batchCommands(batch.tracks, how)) await this.deps.rpc(this.player, c);
				this.refill = (batch.refill as boolean) !== false;
				break;
			case 'randomplay':
				// LMS's Random Mix keeps the queue filled itself.
				if (how === 'load') await this.deps.rpc(this.player, batch.command);
				this.refill = false;
				break;
			case 'ran':
				this.refill = batch.refill === true;
				break;
			default:
				this.refill = false;
		}
	}
}
