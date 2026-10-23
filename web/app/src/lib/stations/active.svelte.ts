// The station playing now, if any: started from Home or the Stations page, kept topped up
// as the selected player's status changes (see refill.ts).
import { untrack } from 'svelte';
import { api, app } from '#lib/state/app.svelte';
import { me } from '#lib/state/me';
import { NO_PLAYER } from '#lib/naming';
import * as cmd from '#lib/lms/commands';
import type { PlayerStatus } from '#lib/lms/types';
import type { Station } from '#lib/api/me';
import { StationRunner, queuedIds } from './refill';

class ActiveStation {
	/** The station started last (shown while it plays). */
	station = $state<Station | null>(null);
	playing = $state(false);
	error = $state('');
	#runner = new StationRunner({
		next: (id, req) => me.next(id, req),
		rpc: (player, c) => api.rpc(player, c),
		queue: async (player) => queuedIds((await api.rpc<PlayerStatus>(player, cmd.queue(0, 200))).playlist_loop),
	});

	/** Start a station on the selected player. */
	async start(station: Station): Promise<boolean> {
		const player = app.playerId;
		this.error = '';
		if (!player) {
			this.error = NO_PLAYER;
			return false;
		}
		this.station = station;
		try {
			await this.#runner.start(station.id, player);
			if (this.#runner.error) this.error = this.#runner.error;
			this.playing = !this.#runner.error;
			app.queueVersion++;
			app.soon();
			return !this.#runner.error;
		} catch (e) {
			this.playing = false;
			this.error = (e as Error).message;
			return false;
		}
	}

	stop() {
		this.#runner.stop();
		this.playing = false;
	}

	/** The selected player's status changed. */
	onStatus(player: string, status: PlayerStatus | null) {
		const managed = this.#runner.active;
		const r = this.#runner.onStatus(player, status);
		if (r) {
			void r.then(() => {
				if (this.#runner.error) this.error = this.#runner.error;
				if (!this.#runner.active) this.playing = false;
				app.queueVersion++;
			});
		}
		if ((managed && !this.#runner.active) || (this.playing && player !== this.#runner.player)) this.playing = false;
	}
}

export const activeStation = new ActiveStation();

// Watch the selected player's status for as long as the app runs.
$effect.root(() => {
	$effect(() => {
		const status = app.now;
		const player = app.playerId;
		untrack(() => activeStation.onStatus(player, status));
	});
});
