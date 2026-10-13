// Rooms (docs/plans/wave2-design.md §4.3): every player this user may control, at a glance.
// While the page is open, each player's status is read every 5 s (and at once when one of
// its events arrives), for at most 20 players; sync groups with each round; the next alarms
// once on opening and again when alarms change. Nothing is read once it closes.
import type { Command } from '$lib/api/rpc';
import type { AlarmListing } from '$lib/api/routine';
import { nextAlarm } from '$lib/api/routine';
import type { LmsEvent } from '$lib/api/events';
import * as cmd from '$lib/lms/commands';
import { syncGroupsFrom } from '$lib/lms/sync';
import type { PlayerInfo, PlayerStatus, SyncGroupsResult } from '$lib/lms/types';

/** Players shown (and polled) at most. */
export const MAX_ROOMS = 20;
export const POLL_MS = 5000;
/** Events for a room are coalesced over this long before its status is read. */
export const EVENT_DELAY_MS = 250;
const PLAYER_ID = /^[0-9a-f]{2}(:[0-9a-f]{2}){5}$/i;

/** One room's status: the current track (with its cover), volume, power, sleep and sync. */
export const roomStatus = (): Command => ['status', '-', 1, 'tags:alcJ'];

export interface RoomsDeps {
	rpc: (player: string, c: Command) => Promise<unknown>;
	alarms: (player: string) => Promise<AlarmListing>;
	/** Whether the page can be seen (polls are skipped while it is hidden). */
	visible?: () => boolean;
	now?: () => Date;
}

export class RoomsPoller {
	/** Status by player id (lower case). */
	status = $state<Record<string, PlayerStatus>>({});
	/** Sync groups (player ids, lower case), groups of one left out. */
	groups = $state<string[][]>([]);
	/** The next alarm by player id, or null for none. */
	alarms = $state<Record<string, Date | null>>({});
	error = $state('');

	#deps: RoomsDeps;
	#players: () => readonly PlayerInfo[] = () => [];
	#timer: ReturnType<typeof setInterval> | undefined;
	#pending = new Map<string, ReturnType<typeof setTimeout>>();
	#running = false;
	/** Bumped at every start and stop: answers from an earlier run are dropped. */
	#run = 0;

	constructor(deps: RoomsDeps) {
		this.#deps = deps;
	}

	get running(): boolean {
		return this.#running;
	}

	/** The players shown: the first MAX_ROOMS of those this user may control. */
	rooms(): PlayerInfo[] {
		return roomsOf(this.#players());
	}

	#ids(): string[] {
		return this.rooms().map((p) => p.playerid.toLowerCase());
	}

	start(players: () => readonly PlayerInfo[]) {
		this.stop();
		this.#players = players;
		this.#running = true;
		const run = ++this.#run;
		void this.pollAll(run);
		void this.loadAlarms(run);
		this.#timer = setInterval(() => {
			if (this.#deps.visible?.() === false) return;
			void this.pollAll(run);
		}, POLL_MS);
	}

	stop() {
		this.#running = false;
		this.#run++;
		clearInterval(this.#timer);
		this.#timer = undefined;
		for (const t of this.#pending.values()) clearTimeout(t);
		this.#pending.clear();
	}

	/** An event from the gateway: read the room it is about soon (all of them after a resync). */
	onEvent(e: LmsEvent | 'resync') {
		if (!this.#running) return;
		if (e === 'resync') {
			void this.pollAll(this.#run);
			return;
		}
		const id = typeof e.player === 'string' ? e.player.toLowerCase() : '';
		if (!id || !this.#ids().includes(id)) return;
		this.soon(id, e.event[0] === 'sync' || e.event[0] === 'client');
	}

	/** Read one room's status shortly (coalescing bursts); `groups` reads sync groups too. */
	soon(id: string, groups = false) {
		if (!this.#running) return;
		const key = id.toLowerCase();
		clearTimeout(this.#pending.get(key));
		const run = this.#run;
		this.#pending.set(
			key,
			setTimeout(() => {
				this.#pending.delete(key);
				void this.#pollOne(run, key);
				if (groups) void this.#pollGroups(run);
			}, EVENT_DELAY_MS),
		);
	}

	async #pollOne(run: number, id: string) {
		try {
			const s = (await this.#deps.rpc(id, roomStatus())) as PlayerStatus | null;
			if (run !== this.#run || !s || typeof s !== 'object') return;
			this.status = { ...this.status, [id]: s };
		} catch {
			/* kept as it was; read again next round */
		}
	}

	async #pollGroups(run: number) {
		if (this.rooms().length < 2) {
			this.groups = [];
			return;
		}
		try {
			const r = (await this.#deps.rpc('', cmd.syncGroups())) as SyncGroupsResult | null;
			if (run !== this.#run) return;
			this.groups = syncGroupsFrom(r, this.#players());
		} catch {
			/* kept as it was */
		}
	}

	/** Read every room's status and the sync groups. */
	async pollAll(run = this.#run) {
		if (run !== this.#run) return;
		const ids = this.#ids();
		await Promise.all([...ids.map((id) => this.#pollOne(run, id)), this.#pollGroups(run)]);
	}

	/** Read every room's next alarm (on opening, and when alarms change). */
	async loadAlarms(run = this.#run) {
		const now = this.#deps.now?.() ?? new Date();
		const out: Record<string, Date | null> = {};
		await Promise.all(
			this.#ids().map(async (id) => {
				try {
					const l = await this.#deps.alarms(id);
					out[id] = l.enabled ? nextAlarm(l.alarms, now) : null;
				} catch {
					out[id] = null;
				}
			}),
		);
		if (run === this.#run) this.alarms = out;
	}
}

/** The rooms shown: well-formed player ids only (nothing else is ever sent), at most MAX_ROOMS. */
export function roomsOf(players: readonly PlayerInfo[]): PlayerInfo[] {
	return players.filter((p) => typeof p?.playerid === 'string' && PLAYER_ID.test(p.playerid)).slice(0, MAX_ROOMS);
}

/** The group a room is in (itself included), or [] when it plays alone. */
export function roomGroup(id: string, groups: readonly string[][]): string[] {
	return groups.find((g) => g.includes(id.toLowerCase())) ?? [];
}

/** "Tue 07:00": when an alarm rings next, short. */
export function alarmText(d: Date | null | undefined, now: Date = new Date()): string {
	if (!d || Number.isNaN(d.getTime())) return '';
	const time = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
	const sameDay = d.getFullYear() === now.getFullYear() && d.getMonth() === now.getMonth() && d.getDate() === now.getDate();
	if (sameDay) return `today ${time}`;
	const tomorrow = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1);
	if (d.getFullYear() === tomorrow.getFullYear() && d.getMonth() === tomorrow.getMonth() && d.getDate() === tomorrow.getDate()) return `tomorrow ${time}`;
	return `${d.toLocaleDateString('en-GB', { weekday: 'short' })} ${time}`;
}

/** Where a keyboard drag goes next: the arrow keys step through the other rooms, round. */
export function stepTarget(ids: readonly string[], from: string, current: string | null, by: 1 | -1): string | null {
	const others = ids.filter((id) => id !== from);
	if (!others.length) return null;
	const at = current ? others.indexOf(current) : -1;
	if (at < 0) return by > 0 ? others[0]! : others.at(-1)!;
	return others[(at + by + others.length) % others.length]!;
}
