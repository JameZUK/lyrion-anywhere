// A stand-in for $lib/state/app.svelte in component tests: reactive like the real one, with an
// `api.rpc` the test scripts (by default: an empty queue, and every command succeeds).
import type { PlayerInfo, PlayerStatus } from '$lib/lms/types';
import { enqueue, type Placement, type Selection } from '$lib/lms/commands';

export type Rpc = (player: string, cmd: (string | number)[]) => Promise<unknown>;
export const calls: { player: string; cmd: (string | number)[] }[] = [];
export const handler: { rpc: Rpc } = { rpc: async () => ({}) };

export const api = {
	rpc: async (player: string, cmd: (string | number)[]) => {
		calls.push({ player, cmd: [...cmd] });
		return handler.rpc(player, cmd);
	},
	art: () => new Promise<string>(() => {}),
};

class FakeApp {
	playerId = $state('');
	players = $state<PlayerInfo[]>([]);
	now = $state<PlayerStatus | null>(null);
	queueVersion = $state(0);
	error = $state('');
	soon = () => {};
	get current() {
		return this.players.find((p) => p.playerid === this.playerId);
	}
	select(id: string) {
		this.playerId = id;
	}
	async send(c: (string | number)[]) {
		await api.rpc(this.playerId, c);
	}
	libraryVersion = $state(0);
	async queueUp(sel: Selection, placement: Placement, playIndex?: number) {
		await this.send(enqueue(sel, placement, playIndex));
	}
}

export const app = new FakeApp();
