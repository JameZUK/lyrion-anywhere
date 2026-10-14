// A stand-in for $lib/state/app.svelte in component tests: reactive like the real one, with an
// `api.rpc` the test scripts (by default: an empty queue, and every command succeeds).
import type { PlayerInfo, PlayerStatus } from '$lib/lms/types';
import { enqueue, type Placement, type Selection } from '$lib/lms/commands';

export type Rpc = (player: string, cmd: (string | number)[]) => Promise<unknown>;
export const calls: { player: string; cmd: (string | number)[] }[] = [];
export type Info = (kind: 'artist' | 'album', id: number) => Promise<unknown>;
export const infoCalls: { kind: 'artist' | 'album'; id: number }[] = [];
/** By default the gateway has no information (as without the plugin). */
export const handler: { rpc: Rpc; info: Info } = { rpc: async () => ({}), info: async () => ({}) };

export const api = {
	rpc: async (player: string, cmd: (string | number)[]) => {
		calls.push({ player, cmd: [...cmd] });
		return handler.rpc(player, cmd);
	},
	art: (_path: string) => new Promise<string>(() => {}),
	info: async (kind: 'artist' | 'album', id: number) => {
		infoCalls.push({ kind, id });
		return handler.info(kind, id);
	},
};

/** The signed-in session, as far as components read it. */
export const session = {
	current: null as null | { user: { username: string; role: string }; device: { id: number; name: string; mac?: string } },
	token: 't',
	subscribe: () => () => {},
	authed: <T,>(fn: (t: string) => Promise<T>) => fn('t'),
	renew: async () => true,
};

export const moves: { source: string; target: string; name: string }[] = [];

class FakeApp {
	playerId = $state('');
	deviceMac = $state('');
	notice = $state('');
	moving = $state('');
	players = $state<PlayerInfo[]>([]);
	now = $state<PlayerStatus | null>(null);
	queueVersion = $state(0);
	error = $state('');
	serverBusy = $state(false);
	soon = () => {};
	/** Event watchers (Rooms); tests call `emit` to deliver one. */
	watchers = new Set<(e: { player: string | null; event: string[] } | 'resync') => void>();
	watchEvents(cb: (e: { player: string | null; event: string[] } | 'resync') => void) {
		this.watchers.add(cb);
		return () => void this.watchers.delete(cb);
	}
	emit(e: { player: string | null; event: string[] } | 'resync') {
		for (const w of this.watchers) w(e);
	}
	get current() {
		return this.players.find((p) => p.playerid === this.playerId);
	}
	get remote() {
		return !!this.playerId && this.playerId !== this.deviceMac && !!this.current;
	}
	get playing() {
		return this.now?.mode === 'play';
	}
	nameOf(id: string) {
		return this.players.find((p) => p.playerid === id)?.name ?? '';
	}
	select(id: string) {
		this.playerId = id;
	}
	deviceFailures = 0;
	deviceFailed() {
		this.deviceFailures++;
	}
	preferDevice() {
		this.playerId = this.deviceMac;
	}
	async moveTo(source: string, target: string, name: string) {
		moves.push({ source, target, name });
		this.playerId = target;
		return true;
	}
	async waitForPlayer(id: string) {
		this.playerId = id;
		return this.players.some((p) => p.playerid === id);
	}
	async send(c: (string | number)[]) {
		await api.rpc(this.playerId, c);
	}
	libraryVersion = $state(0);
	elapsed = $state(0);
	seeks: number[] = [];
	async seek(seconds: number) {
		this.seeks.push(seconds);
		this.elapsed = seconds;
	}
	sleepFade = $state(30);
	armed = $state<null | { end: 'track' | 'album' | 'queue'; players: string[]; endsAt: number; setAt: number }>(null);
	sleepCalls: { choice: unknown; scope: unknown }[] = [];
	async setSleep(choice: unknown, scope: unknown = 'player') {
		this.sleepCalls.push({ choice, scope });
		return '';
	}
	async cancelSleep() {
		this.sleepCalls.push({ choice: 'off', scope: null });
		return '';
	}
	setSleepFade(s: number) {
		this.sleepFade = s;
	}
	onDeviceSleep = (_endsAt: number | null, _fade: number) => {};
	async queueUp(sel: Selection, placement: Placement, playIndex?: number) {
		await this.send(enqueue(sel, placement, playIndex));
	}
}

export const app = new FakeApp();

/** Track analyses (waveforms): none in component tests. */
export const analyses = {
	peek: () => undefined,
	load: () => new Promise<never>(() => {}),
	clear: () => {},
};
