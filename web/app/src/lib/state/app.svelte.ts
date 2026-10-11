// App-wide state: the session, the players the user may control, the selected player's
// status (polled, and refreshed at once on events), and smooth progress between polls.
import { SessionManager, type SessionStatus } from '$lib/api/session';
import { Api } from '$lib/api/rpc';
import { EventsClient, type EventsState } from '$lib/api/events';
import type { Session } from '$lib/api/auth';
import * as cmd from '$lib/lms/commands';
import type { PlayerInfo, PlayersResult, PlayerStatus } from '$lib/lms/types';
import { loadResume } from '$lib/player/resume';
import { canPlayHere } from '$lib/player/capabilities';
import {
	NOTES,
	cleanId,
	decideStart,
	fallbackAfterFailure,
	loadDefault,
	loadTabSelection,
	saveTabSelection,
	type StartChoice,
} from '$lib/player/default-player';
import { refusalText, transfer } from '$lib/lms/transfer';
import { textOf } from '$lib/lms/format';
import { AnalysisClient } from '$lib/analysis/client';
import { ApiError, CLIENT_HEADER, bearer } from '$lib/api/http';

const PLAYER_KEY = 'anywhere.player';
/** The last player other than this device (for falling back when this device can't play). */
const REMOTE_KEY = 'anywhere.player.remote';

function stored(key: string): string {
	try {
		return localStorage.getItem(key) ?? '';
	} catch {
		return ''; // storage blocked
	}
}
const POLL_MS = 2000;

export const session = new SessionManager();
export const api = new Api(session);
/** Track analyses (waveforms) for this session. */
export const analyses = new AnalysisClient((url, signal) =>
	session.authed(async (token) => {
		const r = await fetch(url, { headers: { ...CLIENT_HEADER, ...bearer(token) }, cache: 'no-cache', signal });
		if (r.status === 401) throw new ApiError(401, 'unauthorised');
		return r;
	}),
);

class AppState {
	session = $state<Session | null>(null);
	status = $state<SessionStatus>('unknown');
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
	/** A note for the user about which player is in use (e.g. why not this device). */
	notice = $state('');
	/** Set at sign-in when this device should start its player by itself (the layout does). */
	wantDevice = $state(false);
	/** The player playback is being moved to, while a transfer runs. */
	moving = $state('');

	/** A player to select as soon as LMS lists it (e.g. this device, just connected). */
	#preferred = '';
	#poll: ReturnType<typeof setInterval> | undefined;
	#tick: ReturnType<typeof setInterval> | undefined;
	#statusAt = 0;
	#pending: ReturnType<typeof setTimeout> | undefined;
	#events: EventsClient;
	/** Bumped by every start and stop of the signed-in session. */
	#run = 0;

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

	/** This device's own player id (from the sign-in), or ''. */
	get deviceMac(): string {
		return cleanId(this.session?.device?.mac);
	}

	/** True when controlling a player other than this device. */
	get remote(): boolean {
		return !!this.playerId && this.playerId !== this.deviceMac && !!this.current;
	}

	/** Choose what to control at sign-in or reload (see default-player.ts). */
	#chooseStart() {
		const choice = decideStart({
			setting: loadDefault(this.session?.user?.username ?? ''),
			lastUsed: stored(PLAYER_KEY),
			lastRemote: stored(REMOTE_KEY),
			tabSelection: loadTabSelection(),
			deviceMac: this.deviceMac,
			deviceSupported: canPlayHere(),
		});
		this.#apply(choice);
	}

	#apply(choice: StartChoice) {
		this.wantDevice = choice.use === 'device';
		this.notice = choice.use !== 'device' && choice.note ? NOTES[choice.note] : '';
		if (choice.use === 'device') {
			// LMS lists this device only once its player has connected: hold the choice
			// rather than switching to another player meanwhile.
			this.playerId = this.deviceMac;
			this.#preferred = this.deviceMac;
		} else {
			this.playerId = choice.use === 'player' ? choice.id : '';
			if (this.#preferred === this.deviceMac) this.#preferred = '';
		}
	}

	/**
	 * This device's player couldn't start (or won't, here): stop waiting for it, and control
	 * the last other player instead, saying why.
	 */
	deviceFailed() {
		const mac = this.deviceMac;
		this.wantDevice = false;
		if (this.playerId !== mac || this.current) return;
		this.#apply(fallbackAfterFailure(stored(REMOTE_KEY), mac));
		void this.loadPlayers();
	}

	/** Control this device: hold the choice until LMS lists its player (just starting). */
	preferDevice() {
		const mac = this.deviceMac;
		if (!mac) return;
		this.notice = '';
		if (this.players.some((p) => p.playerid === mac)) this.select(mac);
		else {
			this.playerId = mac;
			this.now = null;
			this.#preferred = mac;
		}
	}

	async #started() {
		this.#chooseStart();
		// This device was the player before a reload: LMS lists it again only once it has
		// reconnected, so hold the choice rather than switching to another player meanwhile.
		if (this.playerId && loadResume()?.mac === this.playerId) this.#preferred = this.playerId;
		const run = ++this.#run;
		this.#events.start();
		await this.loadPlayers();
		// Signed out (or out and in again) while the players loaded: that sign-out already
		// stopped everything, and a newer start owns the timers.
		if (run !== this.#run || this.status !== 'signed-in') return;
		clearInterval(this.#poll);
		clearInterval(this.#tick);
		this.#poll = setInterval(() => {
			if (document.visibilityState === 'visible') void this.refresh();
		}, POLL_MS);
		this.#tick = setInterval(() => this.#advance(), 250);
		document.addEventListener('visibilitychange', this.#onVisible);
	}

	#stopped() {
		this.#run++;
		this.#events.stop();
		clearTimeout(this.#pending);
		clearInterval(this.#poll);
		clearInterval(this.#tick);
		document.removeEventListener('visibilitychange', this.#onVisible);
		api.clearArt();
		analyses.clear();
		this.players = [];
		this.now = null;
		this.notice = '';
		this.wantDevice = false;
		this.#preferred = '';
	}

	#onVisible = () => {
		if (document.visibilityState === 'visible') this.soon();
	};

	async loadPlayers() {
		try {
			const r = await api.rpc<PlayersResult>('', cmd.players());
			this.players = (r.players_loop ?? []).filter((p) => typeof p.playerid === 'string');
			if (this.#preferred && this.players.some((p) => p.playerid === this.#preferred)) {
				const want = this.#preferred;
				this.#preferred = '';
				if (want !== this.playerId) this.select(want);
				else this.soon();
			} else if (!this.current && !this.#preferred) {
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

	/**
	 * Select a player that may not be listed yet: LMS registers a new player a moment after it
	 * connects, so keep checking for a while, and select it the moment it appears.
	 */
	async waitForPlayer(id: string, tries = 20, gapMs = 500): Promise<boolean> {
		this.#preferred = id;
		for (let i = 0; i < tries; i++) {
			await this.loadPlayers();
			if (this.playerId === id && this.current) return true;
			await new Promise((r) => setTimeout(r, gapMs));
		}
		// It never appeared: stop holding out for it, and pick another player as usual.
		if (this.#preferred === id) this.#preferred = '';
		await this.loadPlayers();
		return false;
	}

	/** No usable player: stop polling a remembered one that has gone (LMS drops the
	 *  connection for commands to unknown players, which would show as errors every poll). */
	forget() {
		this.playerId = '';
		this.now = null;
		saveTabSelection('');
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
		if (this.#preferred && this.#preferred !== id) this.#preferred = '';
		saveTabSelection(id);
		try {
			localStorage.setItem(PLAYER_KEY, id);
			if (id !== this.deviceMac) localStorage.setItem(REMOTE_KEY, id);
		} catch {
			/* not essential */
		}
		void this.refresh();
	}

	/**
	 * Move playback from `source` to `target` (docs/PROTOCOL.md, Transfer), then control
	 * `target`. `targetName` is for messages.
	 */
	async moveTo(source: string, target: string, targetName: string): Promise<boolean> {
		if (this.moving) return false;
		this.moving = targetName || 'the other player';
		this.notice = '';
		try {
			const r = await transfer((p, c) => api.rpc(p, c), source, target);
			if (r.ok) {
				this.select(target);
				return true;
			}
			this.notice = 'error' in r ? `Couldn't move playback: ${r.error}` : refusalText(r.refusal, this.moving);
			return false;
		} catch (e) {
			this.notice = `Couldn't move playback: ${(e as Error).message}`;
			return false;
		} finally {
			this.moving = '';
			void this.loadPlayers();
		}
	}

	/** A player's name, as LMS lists it. */
	nameOf(id: string): string {
		return textOf(this.players.find((p) => p.playerid === id)?.name);
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
			this.error = 'Choose a player first: tap the Play on… button by the volume.';
			return;
		}
		await this.send(cmd.enqueue(sel, placement, playIndex));
		this.queueVersion++;
	}

	/** Seek, updating the shown position at once. */
	/** Seek, starting playback there if the player was paused or stopped. */
	async seek(seconds: number) {
		const mode = this.now?.mode;
		this.elapsed = seconds;
		if (this.now) this.now = { ...this.now, time: seconds };
		this.#statusAt = performance.now();
		for (const c of cmd.seekAndPlay(mode, seconds)) await this.send(c);
	}
}

export const app = new AppState();
