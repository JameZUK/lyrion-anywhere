// Queue editing logic, kept free of the DOM so it can be tested on its own.
import type { Command } from '$lib/api/rpc';
import { enqueue, moveInQueue, queue, removeFromQueue, status } from './commands';
import type { PlayerStatus, Track } from './types';
//
// LMS's `playlist move <from> <to>` takes the track out at `from` and puts it back so that it
// ends up at index `to` (in the new order). `reorder` does exactly the same to a local copy, so
// the optimistic list matches what LMS will have once the command lands.

/** A copy of `list` with the item at `from` moved so it ends up at index `to`. */
export function reorder<T>(list: readonly T[], from: number, to: number): T[] {
	const copy = [...list];
	if (!validIndex(list, from) || !validIndex(list, to) || from === to) return copy;
	const [item] = copy.splice(from, 1);
	copy.splice(to, 0, item as T);
	return copy;
}

/** A copy of `list` without the item at `index`. */
export function removeAt<T>(list: readonly T[], index: number): T[] {
	return validIndex(list, index) ? list.filter((_, i) => i !== index) : [...list];
}

/** A copy of `list` with `item` put back at `index` (clamped to the list). */
export function insertAt<T>(list: readonly T[], index: number, item: T): T[] {
	const copy = [...list];
	copy.splice(Math.max(0, Math.min(copy.length, Math.floor(index))), 0, item);
	return copy;
}

function validIndex(list: readonly unknown[], i: number): boolean {
	return Number.isInteger(i) && i >= 0 && i < list.length;
}

/** Where a row sits on screen (from getBoundingClientRect). */
export interface RowBox {
	top: number;
	height: number;
}

/**
 * Where the row dragged from `from` would land if dropped with the pointer at `y`: its index
 * in the new order. That is the number of other rows whose middle is above the pointer.
 */
export function dropIndex(rows: readonly RowBox[], from: number, y: number): number {
	if (!Number.isFinite(y)) return from;
	let to = 0;
	rows.forEach((r, i) => {
		if (i !== from && r.top + r.height / 2 < y) to++;
	});
	return to;
}

/** A swipe must travel this far (in pixels, or this share of the row's width) to remove. */
export const SWIPE_MIN_PX = 80;
export const SWIPE_SHARE = 0.35;

/** Whether a horizontal swipe of `dx` (negative is leftwards) over a row `width` wide removes it. */
export function swipeRemoves(dx: number, width: number): boolean {
	if (!Number.isFinite(dx)) return false;
	const w = Number.isFinite(width) && width > 0 ? width : 0;
	return dx <= -Math.max(SWIPE_MIN_PX, w * SWIPE_SHARE);
}

/**
 * Whether a touch that has moved (dx, dy) is a sideways swipe (ours) rather than a scroll
 * (the browser's): clearly more sideways than up or down, and past a small dead zone.
 */
export function isSwipe(dx: number, dy: number): boolean {
	return Math.abs(dx) > 10 && Math.abs(dx) > Math.abs(dy) * 1.5;
}

/** How an edit reaches LMS (api.rpc). */
export type Rpc = (player: string, cmd: Command) => Promise<unknown>;

/** Undo waits up to this many checks, this far apart, for LMS to add the track back. */
export const UNDO_POLLS = 10;
export const UNDO_POLL_MS = 150;

/**
 * Put a removed library track back where it was (Undo): add it to the end, then move it into
 * place. LMS may apply the add a moment later, so wait until the queue is exactly one longer
 * (the last track is then the one just added) before moving it. Throws if the queue changed
 * meanwhile (the track is then left at the end).
 */
export async function restoreTrack(rpc: Rpc, player: string, trackId: number, index: number, polls = UNDO_POLLS, pollMs = UNDO_POLL_MS): Promise<void> {
	const count = async () => Number(((await rpc(player, status())) as { playlist_tracks?: unknown }).playlist_tracks);
	const before = await count();
	await rpc(player, enqueue({ track: trackId }, 'add'));
	let after = await count();
	for (let i = 0; i < polls && after === before; i++) {
		await new Promise((r) => setTimeout(r, pollMs));
		after = await count();
	}
	if (!Number.isInteger(before) || after !== before + 1) {
		throw new Error('the queue changed meanwhile, so it was added at the end');
	}
	if (before > index) await rpc(player, moveInQueue(before, index));
}

// --- checked edits ----------------------------------------------------------------------
// LMS edits the queue by position only (`playlist delete <index>`; `playlistcontrol cmd:delete
// track_id:…` would take out every copy of the track). The queue can change under the user
// (another app, a station topping it up, the track ending), so an edit first checks that the
// track it was meant for is still at that position, and refuses otherwise.

/** The queue no longer has the expected track at that position: nothing was changed. */
export class QueueChanged extends Error {
	constructor() {
		super('the queue changed meanwhile, so nothing was changed');
		this.name = 'QueueChanged';
	}
}

/** Whether two queue entries are the same track (by id, or by title for entries without one). */
export function sameTrack(a: Track | undefined, b: Track | undefined): boolean {
	if (!a || !b) return false;
	if (a.id !== undefined || b.id !== undefined) return a.id === b.id;
	return (a.title ?? '') === (b.title ?? '');
}

/** Throws QueueChanged unless `track` is (still) at `index` in `player`'s queue. */
export async function checkAt(rpc: Rpc, player: string, index: number, track: Track): Promise<void> {
	if (!Number.isInteger(index) || index < 0) throw new QueueChanged();
	const r = (await rpc(player, queue(index, 1))) as PlayerStatus | null;
	const at = r?.playlist_loop?.[0];
	const pos = at?.['playlist index'];
	if (!sameTrack(at, track) || (pos !== undefined && Number(pos) !== index)) throw new QueueChanged();
}

/** Remove `track` from position `index`, if it is still there. */
export async function removeChecked(rpc: Rpc, player: string, index: number, track: Track): Promise<void> {
	await checkAt(rpc, player, index, track);
	await rpc(player, removeFromQueue(index));
}

/** Move `track` from position `from` so it ends up at `to`, if it is still at `from`. */
export async function moveChecked(rpc: Rpc, player: string, from: number, to: number, track: Track): Promise<void> {
	await checkAt(rpc, player, from, track);
	await rpc(player, moveInQueue(from, to));
}
