// App-wide state: the session, the players the user may control, the selected player's
// status (polled, and refreshed at once on events), and smooth progress between polls.
import { SessionManager } from '$lib/api/session';
import { Api } from '$lib/api/rpc';
import { EventsClient, type EventsState } from '$lib/api/events';
import type { Session } from '$lib/api/auth';
import * as cmd from '$lib/lms/commands';
import type { PlayerInfo, PlayersResult, PlayerStatus } from '$lib/lms/types';

const PLAYER_KEY = 'anywhere.player';
const POLL_MS = 2000;

export const session = new SessionManager();
export const api = new Api(session);

class AppState {
	session = $state<Session | null>(null);
	status = $state<'unknown' | 'signed-out' | 'signed-in'>('unknown');
	players = $state<PlayerInfo[]>([]);
	playerId = $state<string>('');
	now = $state<PlayerStatus | null>(null);
	/** Elapsed seconds, advanced locally between polls while playing. */
	elapsed = $state(0);
	events = $state<EventsState>('stopped');
	error = $state('');
	/** Bumped when the library changes (rescan), so views can reload. */
	libraryVersion = $state(0);
	/** Bumped when the queue may have changed. */
	queueVersion = $state(0);

	#poll: ReturnType<typeof setInterval> | undefined;
	#tick: ReturnType<typeof setInterval> | undefined;
	#statusAt = 0;
	#pending: ReturnType<typeof setTimeout> | undefined;
	#events: EventsClient;

	constructor() {
		this.#events = new EventsClient(session, {
			onEvent: (e) => {
				if (e.player === null) {
					if (e.event[0] === 'rescan') this.libraryVersion++;
					return;
				}
				if (e.event[0] === 'client' || e.event[0] === 'power') void this.loadPlayers();
				if (e.player === this.playerId) {
					if (e.event[0] === 'playlist') this.queueVersion++;
					this.soon();
				}
			},
			onResync: () => {
				void this.loadPlayers();
				this.queueVersion++;
				this.soon();
			},
			onState: (s) => (this.events = s),
		});
		session.subscribe((s, st) => {
			const was = this.status;
			this.session = s;
			this.status = st;
			if (st === 'signed-in' && was !== 'signed-in') this.#started();
			if (st === 'signed-out' && was === 'signed-in') this.#stopped();
		});
	}

	get current(): PlayerInfo | undefined {
		return this.players.find((p) => p.playerid === this.playerId);
	}

	/** Admins may change shared data (playlists); listeners only play it. */
	get isAdmin(): boolean {
		return this.session?.user.role === 'admin';
	}

	get playing(): boolean {
		return this.now?.mode === 'play';
	}

	async #started() {
		try {
			this.playerId = localStorage.getItem(PLAYER_KEY) ?? '';
		} catch {
			/* storage blocked */
		}
		this.#events.start();
		await this.loadPlayers();
		this.#poll = setInterval(() => {
			if (document.visibilityState === 'visible') void this.refresh();
		}, POLL_MS);
		this.#tick = setInterval(() => this.#advance(), 250);
		document.addEventListener('visibilitychange', this.#onVisible);
	}

	#stopped() {
		this.#events.stop();
		clearInterval(this.#poll);
		clearInterval(this.#tick);
		document.removeEventListener('visibilitychange', this.#onVisible);
		api.clearArt();
		this.players = [];
		this.now = null;
	}

	#onVisible = () => {
		if (document.visibilityState === 'visible') this.soon();
	};

	async loadPlayers() {
		try {
			const r = await api.rpc<PlayersResult>('', cmd.players());
			this.players = (r.players_loop ?? []).filter((p) => typeof p.playerid === 'string');
			if (!this.current) {
				// Keep the remembered player if it's still there; else the first one playing,
				// else the first one.
				const pick = this.players.find((p) => p.isplaying) ?? this.players[0];
				if (pick) this.select(pick.playerid);
				else this.forget();
			} else {
				this.soon();
			}
		} catch (e) {
			this.error = (e as Error).message;
		}
	}

	/** No usable player: stop polling a remembered one that has gone (LMS drops the
	 *  connection for commands to unknown players, which would show as errors every poll). */
	forget() {
		this.playerId = '';
		this.now = null;
		try {
			localStorage.removeItem(PLAYER_KEY);
		} catch {
			/* not essential */
		}
	}

	select(id: string) {
		this.playerId = id;
		this.now = null;
		this.queueVersion++;
		try {
			localStorage.setItem(PLAYER_KEY, id);
		} catch {
			/* not essential */
		}
		void this.refresh();
	}

	/** Refresh status shortly (coalesces bursts of events). */
	soon() {
		clearTimeout(this.#pending);
		this.#pending = setTimeout(() => void this.refresh(), 120);
	}

	async refresh() {
		const id = this.playerId;
		if (!id || !this.current) return;
		try {
			const s = await api.rpc<PlayerStatus>(id, cmd.status());
			if (id !== this.playerId) return; // switched meanwhile
			this.now = s;
			this.elapsed = typeof s.time === 'number' ? s.time : 0;
			this.#statusAt = performance.now();
			this.error = '';
		} catch (e) {
			this.error = (e as Error).message;
		}
	}

	#advance() {
		const s = this.now;
		if (!s || s.mode !== 'play' || typeof s.time !== 'number') return;
		const t = s.time + (performance.now() - this.#statusAt) / 1000;
		this.elapsed = s.duration ? Math.min(t, s.duration) : t;
	}

	/** Send a command to the selected player, then refresh its status. */
	async send(c: (string | number)[]) {
		const id = this.playerId;
		if (!id || !this.current) return;
		try {
			await api.rpc(id, c);
		} catch (e) {
			this.error = (e as Error).message;
		}
		this.soon();
	}

	/** Play, play next or add a selection on the selected player. */
	async queueUp(sel: cmd.Selection, placement: cmd.Placement, playIndex?: number) {
		if (!this.playerId) {
			this.error = 'Choose a player first (bottom right, or Settings › This device).';
			return;
		}
		await this.send(cmd.enqueue(sel, placement, playIndex));
		this.queueVersion++;
	}

	/** Seek, updating the shown position at once. */
	async seek(seconds: number) {
		this.elapsed = seconds;
		if (this.now) this.now = { ...this.now, time: seconds };
		this.#statusAt = performance.now();
		await this.send(cmd.seek(seconds));
	}
}

export const app = new AppState();
