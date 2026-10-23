// Transfer: move what one player is playing (its queue, position and play state) to another,
// start it there, and stop the first. The recipe is documented in docs/PROTOCOL.md
// ("Transfer"); the native apps follow the same one.
//
// 1. Group, then leave (preferred). `<source> sync <target>` makes the target join the
//    source's group: LMS gives it the group's queue (radio and app items included), with the
//    same track, position, shuffle order and repeat, and starts it in step. `<source> sync -`
//    then takes the source out: it stops (keeping a copy of the queue), while the target
//    carries on without a gap. The target's own queue is replaced.
// 2. Copy the list (fallback, when LMS won't group the two): read the source's queue, load
//    the same tracks on the target by id, jump to the track and time, then stop the source.
//    Only library tracks can be copied like this (the gateway never reveals URLs).
import type { Command } from '#lib/api/rpc';
import * as cmd from './commands';

/** The gateway pages at most this many queue entries at once; longer queues can't be copied. */
export const MAX_COPY = 500;

/** What the source says about itself (from `status - 1`). */
export interface SourceStatus {
	mode?: string;
	time?: number | string;
	playlist_tracks?: number | string;
	playlist_cur_index?: number | string;
	'playlist repeat'?: number | string;
}

/** A queue entry (from `status 0 N`). */
export interface QueueEntry {
	id?: unknown;
	remote?: unknown;
}

export type Step =
	| { on: 'source' | 'target'; cmd: Command }
	/** Check that the target has joined the source's group before going on. */
	| { verify: 'grouped' };

export type Refusal = 'same' | 'empty' | 'remote' | 'too-long' | 'not-grouped';

export type Plan = { ok: true; steps: Step[] } | { ok: false; refusal: Refusal };

const num = (v: unknown): number => {
	const n = typeof v === 'number' ? v : typeof v === 'string' && v.trim() !== '' ? Number(v) : NaN;
	return Number.isFinite(n) ? n : NaN;
};
const count = (v: unknown) => {
	const n = num(v);
	return n >= 0 ? Math.floor(n) : 0;
};

/** How the target should end up playing, given how the source was. */
function startSteps(mode: unknown): Step[] {
	if (mode === 'pause') return [{ on: 'target', cmd: cmd.pause(false) }];
	if (mode === 'play') return [];
	return [{ on: 'target', cmd: cmd.play() }]; // stopped: start it, as asked
}

/**
 * Recipe 1: group, then leave. Throws (RangeError) for malformed player ids: nothing is
 * sent for them.
 */
export function planGroupTransfer(source: string, target: string, s: SourceStatus | null | undefined): Plan {
	const from = cmd.playerId(source);
	const to = cmd.playerId(target);
	if (from === to) return { ok: false, refusal: 'same' };
	if (!s || count(s.playlist_tracks) === 0) return { ok: false, refusal: 'empty' };
	return {
		ok: true,
		steps: [
			// A target that is off joins the group but stays silent: switch it on first.
			{ on: 'target', cmd: cmd.power(true) },
			// The target joins the source's group (leaving any group it was in) and takes its queue.
			{ on: 'source', cmd: cmd.syncWith(to) },
			{ verify: 'grouped' },
			// The source leaves: it stops, and the target carries on.
			{ on: 'source', cmd: cmd.unsync() },
			{ on: 'source', cmd: cmd.stop() },
			...startSteps(s.mode),
		],
	};
}

/**
 * Recipe 2: copy the list. `queue` is the source's whole queue in play order (`status 0 N`),
 * `s` its status read just before (with `time` already advanced to now, if playing).
 */
export function planCopyTransfer(source: string, target: string, s: SourceStatus | null | undefined, queue: readonly QueueEntry[]): Plan {
	const from = cmd.playerId(source);
	const to = cmd.playerId(target);
	if (from === to) return { ok: false, refusal: 'same' };
	const total = s ? count(s.playlist_tracks) : 0;
	if (total === 0) return { ok: false, refusal: 'empty' };
	if (total > MAX_COPY || queue.length < total) return { ok: false, refusal: 'too-long' };
	const ids: number[] = [];
	for (const e of queue.slice(0, total)) {
		// Radio, app and other remote items have no library id (LMS gives them negative ones).
		if (e.remote === 1 || e.remote === '1') return { ok: false, refusal: 'remote' };
		try {
			ids.push(cmd.id(e.id));
		} catch {
			return { ok: false, refusal: 'remote' };
		}
	}
	const index = Math.min(count(s!.playlist_cur_index), ids.length - 1);
	const time = num(s!.time);
	const repeat = count(s!['playlist repeat']);
	const chunks = cmd.chunkIds(ids);
	const first = chunks[0]!;
	const steps: Step[] = [
		{ on: 'target', cmd: cmd.power(true) },
		// The list is copied in play order, so the target must not shuffle it again.
		{ on: 'target', cmd: cmd.shuffle(0) },
		{ on: 'target', cmd: cmd.repeat((repeat <= 2 ? repeat : 0) as 0 | 1 | 2) },
		{ on: 'target', cmd: cmd.loadTracks(first, 'load', index < first.length ? index : 0) },
		...chunks.slice(1).map((c): Step => ({ on: 'target', cmd: cmd.loadTracks(c, 'add') })),
	];
	if (index >= first.length) steps.push({ on: 'target', cmd: cmd.jump(index) });
	// A second or two in is as good as the start.
	if (time >= 2) steps.push({ on: 'target', cmd: cmd.seek(Math.floor(time)) });
	steps.push({ on: 'source', cmd: cmd.stop() });
	return { ok: true, steps };
}

type Rpc = <T = Record<string, unknown>>(player: string, cmd: Command) => Promise<T>;

export interface TransferOptions {
	/** Waits between checks (tests pass a no-op). */
	sleep?: (ms: number) => Promise<void>;
	now?: () => number;
}

export type TransferResult = { ok: true; method: 'group' | 'copy' } | { ok: false; refusal: Refusal } | { ok: false; error: string };

interface TargetStatus {
	sync_master?: unknown;
	sync_slaves?: unknown;
	playlist_tracks?: unknown;
}

/** Has `target` joined `source`'s group with the same queue? */
export function grouped(source: string, s: TargetStatus | null | undefined, tracks: number): boolean {
	if (!s) return false;
	const members = [s.sync_master, s.sync_slaves]
		.filter((v): v is string => typeof v === 'string')
		.join(',')
		.toLowerCase()
		.split(',');
	return members.includes(source.toLowerCase()) && count(s.playlist_tracks) === tracks;
}

async function run(rpc: Rpc, source: string, target: string, steps: Step[], verify: () => Promise<boolean>): Promise<boolean> {
	for (const step of steps) {
		if ('verify' in step) {
			if (!(await verify())) return false;
			continue;
		}
		await rpc(step.on === 'source' ? source : target, step.cmd);
	}
	return true;
}

/**
 * Move playback from `source` to `target` (both players this user may control: the gateway
 * refuses anything else). Tries recipe 1, and recipe 2 if LMS won't group them.
 */
export async function transfer(rpc: Rpc, source: string, target: string, o: TransferOptions = {}): Promise<TransferResult> {
	const sleep = o.sleep ?? ((ms: number) => new Promise<void>((r) => setTimeout(r, ms)));
	const now = o.now ?? (() => Date.now());
	let joined = false;
	try {
		const s = await rpc<SourceStatus>(source, cmd.status());
		const plan = planGroupTransfer(source, target, s);
		if (!plan.ok) return plan;
		const tracks = count(s.playlist_tracks);
		const done = await run(rpc, source, target, plan.steps, async () => {
			joined = true;
			for (let i = 0; i < 6; i++) {
				if (grouped(source, await rpc<TargetStatus>(target, cmd.status()), tracks)) return true;
				await sleep(250);
			}
			return false;
		});
		if (done) return { ok: true, method: 'group' };

		// LMS didn't group them: take the target out again, and copy the list instead.
		await rpc(target, cmd.unsync()).catch(() => {});
		joined = false;
		const fresh = await rpc<SourceStatus>(source, cmd.status());
		const readAt = now();
		const q = await rpc<{ playlist_loop?: QueueEntry[] }>(source, cmd.queue(0, MAX_COPY));
		const t = num(fresh.time);
		const time = fresh.mode === 'play' && Number.isFinite(t) ? t + (now() - readAt) / 1000 : t;
		const copy = planCopyTransfer(source, target, { ...fresh, time }, Array.isArray(q.playlist_loop) ? q.playlist_loop : []);
		if (!copy.ok) return copy.refusal === 'remote' || copy.refusal === 'too-long' ? copy : { ok: false, refusal: 'not-grouped' };
		await run(rpc, source, target, copy.steps, async () => true);
		return { ok: true, method: 'copy' };
	} catch (e) {
		// Never leave the two grouped by accident.
		if (joined) await rpc(target, cmd.unsync()).catch(() => {});
		return { ok: false, error: (e as Error).message };
	}
}

/** What to tell the user when a transfer can't be done. */
export function refusalText(r: Refusal, target: string): string {
	switch (r) {
		case 'same':
			return `${target} is already playing this.`;
		case 'empty':
			return 'Nothing to move: the queue is empty.';
		case 'remote':
			return `${target} can't take over radio or app items. Start them on ${target} instead.`;
		case 'too-long':
			return `This queue is too long to move to ${target} (at most ${MAX_COPY} tracks).`;
		case 'not-grouped':
			return `Couldn't move playback to ${target}.`;
	}
}
