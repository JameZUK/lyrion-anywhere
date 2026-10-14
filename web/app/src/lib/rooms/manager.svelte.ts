// The one player manager (Wave 3.5, item 13): what Play on… and Rooms both show and do for
// every player this user may control. Statuses come from a RoomsPoller, read only while one of
// them is open; the actions (play/pause, volume, power, sync, moving playback) are the same in
// both, and each needs the server (nothing is sent while it is out of reach or busy).
import type { Command } from '$lib/api/rpc';
import type { RowAction } from '$lib/components/RowMenu.svelte';
import * as cmd from '$lib/lms/commands';
import { textOf } from '$lib/lms/format';
import { groupLabel, syncToggle } from '$lib/lms/sync';
import type { PlayerInfo, PlayerStatus } from '$lib/lms/types';
import { isNetworkError } from '$lib/state/reachability';
import { RoomsPoller, roomGroup, type RoomsDeps } from './rooms.svelte';

/** What the manager needs from the app (the real one, or a test's). */
export interface ManagerApp {
	readonly playerId: string;
	readonly canQueue: boolean;
	readonly players: readonly PlayerInfo[];
	readonly moving: string;
	readonly current: PlayerInfo | undefined;
	readonly now: PlayerStatus | null;
	select(id: string): void;
	soon(): void;
	moveTo(source: string, target: string, name: string): Promise<boolean>;
}

export interface ManagerDeps extends RoomsDeps {
	app: ManagerApp;
	/** Move playback to this device (starting its player, in the tap). */
	bringHere: () => Promise<boolean>;
	isThisDevice: (id: string) => boolean;
	/** Why an item is unavailable, when the connection is the reason (controls.why). */
	why?: (disabled: boolean) => { title?: string; 'aria-describedby'?: string };
}

export const nameOf = (p: PlayerInfo) => textOf(p.name) || 'Player';
export const keyOf = (p: PlayerInfo) => p.playerid.toLowerCase();

export class PlayerManager {
	readonly poller: RoomsPoller;
	/** What happened, for screen readers (Rooms shows it in a live region). */
	notice = $state('');
	error = $state('');
	#deps: ManagerDeps;

	constructor(deps: ManagerDeps) {
		this.#deps = deps;
		this.poller = new RoomsPoller(deps);
	}

	get #app() {
		return this.#deps.app;
	}

	start(players: () => readonly PlayerInfo[]) {
		this.poller.start(players);
	}

	stop() {
		this.poller.stop();
	}

	status(p: PlayerInfo): PlayerStatus | undefined {
		return this.poller.status[keyOf(p)];
	}

	/** Switched on (as far as is known: on until a status says otherwise). */
	isOn(p: PlayerInfo): boolean {
		return Number(this.status(p)?.power ?? 1) !== 0;
	}

	isPlaying(p: PlayerInfo): boolean {
		return this.isOn(p) && this.status(p)?.mode === 'play';
	}

	/** "with Lounge", for a player in a group, naming the others. */
	groupNote(p: PlayerInfo): string {
		const id = keyOf(p);
		const g = roomGroup(id, this.poller.groups).filter((x) => x !== id);
		return g.length ? `with ${groupLabel(g, this.#app.players)}` : '';
	}

	/** Something to move: the player in use has a queue. */
	get canMove(): boolean {
		return !!this.#app.current && Number(this.#app.now?.playlist_tracks ?? 0) > 0 && !this.#app.moving;
	}

	async send(p: PlayerInfo, c: Command, groups = false) {
		if (!this.#app.canQueue) return;
		this.error = '';
		try {
			await this.#deps.rpc(p.playerid, c);
		} catch (e) {
			if (!isNetworkError(e)) this.error = `${nameOf(p)}: ${(e as Error).message}`;
		}
		this.poller.soon(keyOf(p), groups);
		if (p.playerid === this.#app.playerId) this.#app.soon();
	}

	playPause(p: PlayerInfo) {
		const s = this.status(p);
		void this.send(p, this.isPlaying(p) ? cmd.pause(true) : s?.mode === 'pause' ? cmd.pause(false) : cmd.play());
	}

	power(p: PlayerInfo) {
		void this.send(p, cmd.power(!this.isOn(p)));
	}

	volume(p: PlayerInfo, v: number) {
		void this.send(p, cmd.volume(v));
	}

	/** `joiner` (an id, lower case) joins `host`'s group (and takes its queue). */
	async syncInto(host: string, joiner: string) {
		const rooms = this.poller.rooms();
		const h = rooms.find((p) => keyOf(p) === host);
		const j = rooms.find((p) => keyOf(p) === joiner);
		if (!h || !j || host === joiner || !this.#app.canQueue) return;
		let t;
		try {
			t = syncToggle(h.playerid, j.playerid, true);
		} catch {
			return; // not a well-formed player id: never sent
		}
		this.error = '';
		try {
			await this.#deps.rpc(t.player, t.cmd);
			this.notice = `${nameOf(j)} now plays in sync with ${nameOf(h)}.`;
		} catch (e) {
			this.error = `Couldn't sync ${nameOf(j)} with ${nameOf(h)}: ${(e as Error).message}`;
		}
		this.poller.soon(host, true);
		this.poller.soon(joiner);
		this.#app.soon();
	}

	async unsync(p: PlayerInfo) {
		await this.send(p, cmd.unsync(), true);
		this.notice = `${nameOf(p)} now plays on its own.`;
	}

	/** Move playback from the player in use to `p` (this device: start its player first). */
	move(p: PlayerInfo) {
		if (!this.#app.canQueue) return;
		void (this.#deps.isThisDevice(p.playerid) ? this.#deps.bringHere() : this.#app.moveTo(this.#app.playerId, p.playerid, nameOf(p)));
	}

	/**
	 * The menu for a player (Play on…'s rows and Rooms' cards share it). `extra`: the place's
	 * own items, first (Rooms: Play on <player>, Sync with…).
	 */
	actionsFor(p: PlayerInfo, opts: { alarms?: () => void; extra?: RowAction[] } = {}): RowAction[] {
		const name = nameOf(p);
		const off = !this.#app.canQueue;
		const why = this.#deps.why?.(off);
		const out: RowAction[] = [...(opts.extra ?? [])];
		if (this.canMove && p.playerid !== this.#app.playerId) out.push({ label: `Move playback to ${this.#deps.isThisDevice(p.playerid) ? 'this device' : name}`, icon: 'transfer', run: () => this.move(p), disabled: off, why, id: 'move' });
		if (this.status(p) && this.isOn(p)) out.push({ label: `${this.isPlaying(p) ? 'Pause' : 'Play'} ${name}`, icon: this.isPlaying(p) ? 'pause' : 'play', run: () => this.playPause(p), disabled: off, why, id: 'play-pause' });
		if (roomGroup(keyOf(p), this.poller.groups).length > 1) out.push({ label: 'Play on its own', icon: 'unlink', run: () => void this.unsync(p), disabled: off, why, id: 'unsync' });
		if (opts.alarms) out.push({ label: 'Alarms', icon: 'alarm', run: opts.alarms, id: 'alarms' });
		if (this.status(p)) out.push({ label: `Switch ${name} ${this.isOn(p) ? 'off' : 'on'}`, icon: 'power', run: () => this.power(p), disabled: off, why, id: 'power' });
		return out;
	}
}
