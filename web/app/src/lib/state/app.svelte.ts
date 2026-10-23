// App-wide state: the session, the players the user may control, the selected player's
// status (polled, and refreshed at once on events), and smooth progress between polls.
import { SessionManager, type SessionStatus } from '#lib/api/session';
import { Api } from '#lib/api/rpc';
import { EventsClient, type EventsState, type LmsEvent, type LmsHealth } from '#lib/api/events';
import type { Session } from '#lib/api/auth';
import * as cmd from '#lib/lms/commands';
import { NO_PLAYER } from '#lib/naming';
import type { PlayerInfo, PlayersResult, PlayerStatus } from '#lib/lms/types';
import { clearResume, loadResume } from '#lib/player/resume';
import { canPlayHere } from '#lib/player/capabilities';
import {
	NOTES,
	cleanId,
	decideStart,
	fallbackAfterFailure,
	loadDefault,
	loadTabSelection,
	saveTabSelection,
	type StartChoice,
} from '#lib/player/default-player';
import { refusalText, transfer } from '#lib/lms/transfer';
import { textOf } from '#lib/lms/format';
import { AnalysisClient } from '#lib/analysis/client';
import { ApiError, BUSY_MESSAGE, CLIENT_HEADER, bearer, isBusy } from '#lib/api/http';
import { BusyGate, POLL_MS, Single } from './busy';
import { ReachabilityTracker, isNetworkError, type Reachability } from './reachability';
import {
	armedOver,
	choiceSeconds,
	loadFade,
	rearmSeconds,
	remainingSeconds,
	saveFade,
	sleepLeft,
	sleepTargets,
	type Armed,
	type SleepChoice,
	type SleepScope,
} from '#lib/lms/sleep';
import type { Track } from '#lib/lms/types';
import { upNextTracker } from '#lib/queue/upnext.svelte';
import { stateOf } from '#lib/queue/tracker';
import { forgetLastKnown, loadLastKnown, saveLastKnown } from './last-known';

const PLAYER_KEY = 'anywhere.player';
/** How often the selected player's state is noted for the next page load (and on leaving). */
const REMEMBER_MS = 10_000;
/** The last player other than this device (for falling back when this device can't play). */
const REMOTE_KEY = 'anywhere.player.remote';

function stored(key: string): string {
	try {
		return localStorage.getItem(key) ?? '';
	} catch {
		return ''; // storage blocked
	}
}

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
	/** Signed in as far as this device knows, waiting for the server to confirm it (see session.ts). */
	provisional = $state(false);
	players = $state<PlayerInfo[]>([]);
	playerId = $state<string>('');
	now = $state<PlayerStatus | null>(null);
	/** Elapsed seconds, advanced locally between polls while playing. */
	elapsed = $state(0);
	events = $state<EventsState>('stopped');
	error = $state('');
	/** The music server isn't answering for now (busy): a calm status, not an error. */
	serverBusy = $state(false);
	/**
	 * Can the server be reached (see reachability.ts)? Controls that need it are disabled
	 * while it is anything but `online`.
	 */
	reach = $state<Reachability>('online');
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
	/** The sleep timer's fade-out (seconds; per user, kept in this browser for now). */
	sleepFade = $state(30);
	/** A sleep timer set to an end of the queue, kept in step with it (see sleep.ts). */
	armed = $state<Armed | null>(null);
	/** The queue after the current track, for the armed timer, and what it was read for. */
	#upcoming: { key: string; tracks: Track[] } | null = null;
	/** Fades this device's own output before its timer ends (set by the layout: device.setSleepFade). */
	onDeviceSleep: (endsAt: number | null, fadeS: number) => void = () => {};
	#sleepBusy = false;

	/** A player to select as soon as LMS lists it (e.g. this device, just connected). */
	#preferred = '';
	#poll: ReturnType<typeof setInterval> | undefined;
	#tick: ReturnType<typeof setInterval> | undefined;
	#statusAt = 0;
	#pending: ReturnType<typeof setTimeout> | undefined;
	#events: EventsClient;
	#busy = new BusyGate();
	#reachability = new ReachabilityTracker();
	#reachTimer: ReturnType<typeof setTimeout> | undefined;
	/** The last request failed for want of a connection. */
	#requestsFailing = false;
	/** This device is the player and its socket is down (set by the layout). */
	#deviceDown = false;
	/** Status requests, one at a time: a poll never stacks on one still waiting. */
	#refresher = new Single(
		() => this.#refreshNow(),
		() => !this.#busy.busy,
	);
	/** Views that follow every player's events (Rooms), with 'resync' when some were missed. */
	#watchers = new Set<(e: LmsEvent | 'resync') => void>();
	/** Bumped by every start and stop of the signed-in session. */
	#run = 0;
	/** `now` is the last state seen before this page loaded, not fresh from the server. */
	#stale = false;
	#rememberedAt = 0;

	constructor() {
		this.#events = new EventsClient(session, {
			onEvent: (e) => {
				for (const w of this.#watchers) w(e);
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
				for (const w of this.#watchers) w('resync');
				void this.loadPlayers();
				this.queueVersion++;
				this.soon();
			},
			onState: (s) => {
				this.events = s;
				this.#reassess();
			},
			onLms: (h) => this.#onLms(h),
		});
		api.onHealth = (e) => {
			this.#reached(true);
			if (e) this.#markBusy(e.retryAfter === undefined ? null : Math.min(e.retryAfter, 300));
			else this.#markOk();
		};
		api.onUnreachable = () => this.#reached(false);
		if (typeof window !== 'undefined') {
			addEventListener('online', () => this.#reassess());
			addEventListener('offline', () => this.#reassess());
		}
		session.subscribe((s, st) => {
			const was = this.status;
			const wasProvisional = this.provisional;
			this.session = s;
			this.status = st;
			this.provisional = session.provisional;
			if (st === 'signed-in' && was !== 'signed-in') this.#started();
			if (st === 'signed-out' && was === 'signed-in') this.#stopped();
			if (st === 'signed-out') {
				// Nothing of this person's lingers for the next one to sign in on this browser.
				forgetLastKnown();
				this.#forgetPlayers();
			}
			// The server has confirmed the sign-in this page started with: catch up at once.
			if (st === 'signed-in' && wasProvisional && !session.provisional) {
				void this.loadPlayers();
				this.soon();
			}
		});
	}

	/** Follow every event (any player's), until the returned function is called. */
	watchEvents(cb: (e: LmsEvent | 'resync') => void): () => void {
		this.#watchers.add(cb);
		return () => void this.#watchers.delete(cb);
	}

	get current(): PlayerInfo | undefined {
		return this.players.find((p) => p.playerid === this.playerId);
	}

	/** Admins may change shared data (playlists); listeners only play it. */
	get isAdmin(): boolean {
		return this.session?.user.role === 'admin';
	}

	/**
	 * Can library "play" actions (play, play next, add to queue, play a playlist, station or
	 * favourite…) be used now? They all need the server: every one of them is disabled with
	 * this while it is out of reach or busy.
	 */
	get canQueue(): boolean {
		return this.reach === 'online';
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
		this.sleepFade = loadFade(this.session?.user?.username ?? '');
		this.#chooseStart();
		this.#restoreLastKnown();
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
			// While LMS is busy, ask less often (the events stream says when it answers again).
			if (document.visibilityState === 'visible' && this.#busy.mayPoll(Date.now())) void this.#refresher.poll();
		}, POLL_MS);
		this.#tick = setInterval(() => this.#advance(), 250);
		document.addEventListener('visibilitychange', this.#onVisible);
	}

	/**
	 * Show what the chosen player was last seen doing until the server says otherwise: a page
	 * loaded while it is busy or out of reach then shows that (greyed, with the connection
	 * message) rather than nothing.
	 */
	#restoreLastKnown() {
		const last = loadLastKnown(this.session?.user?.username ?? '', this.playerId);
		if (!last || this.players.length) return;
		this.players = [last.player];
		this.now = last.now;
		this.elapsed = typeof last.now.time === 'number' ? last.now.time : 0;
		this.#stale = true;
	}

	/** Note what the selected player is doing, for the next page load (see last-known.ts). */
	rememberNow() {
		const user = this.session?.user?.username;
		const player = this.current;
		if (!user || !player || !this.now || this.#stale) return;
		this.#rememberedAt = Date.now();
		saveLastKnown({ user, player: $state.snapshot(player), now: { ...$state.snapshot(this.now), time: this.elapsed } });
	}

	#stopped() {
		this.#run++;
		this.#stale = false;
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
		this.armed = null;
		this.#upcoming = null;
		this.#busy.markOk();
		this.serverBusy = false;
		this.#requestsFailing = false;
		this.#deviceDown = false;
		this.#reachability.reset();
		this.#reassess();
		upNextTracker.reset();
	}

	/** A request reached the server (true), or failed for want of a connection (false). */
	#reached(ok: boolean) {
		if (this.#requestsFailing === !ok) return;
		this.#requestsFailing = !ok;
		this.#reassess();
	}

	/** This device is the player and its connection to the gateway is down (or back). */
	setDeviceDown(down: boolean) {
		if (this.#deviceDown === down) return;
		this.#deviceDown = down;
		this.#reassess();
	}

	/** Work out `reach` again from everything the app sees. */
	#reassess() {
		clearTimeout(this.#reachTimer);
		const online = typeof navigator === 'undefined' || navigator.onLine !== false;
		const r = this.#reachability.update(
			{ browserOnline: online, events: this.events, requestsFailing: this.#requestsFailing, deviceDown: this.#deviceDown, busy: this.serverBusy },
			Date.now(),
		);
		const was = this.reach;
		this.reach = r.state;
		if (r.recheckAt !== null) this.#reachTimer = setTimeout(() => this.#reassess(), Math.max(0, r.recheckAt - Date.now()));
		// Back: the per-command errors seen meanwhile are explained; catch up at once.
		if (was === 'unreachable' && r.state !== 'unreachable' && this.status === 'signed-in') {
			this.error = '';
			void this.loadPlayers();
			this.soon();
		}
	}

	/** Report a failure as an error, unless it is the server being out of reach (or busy). */
	#failed(e: unknown) {
		if (isBusy(e) || isNetworkError(e)) return;
		this.error = (e as Error).message;
	}

	/** From the events stream: LMS stopped answering, or answers again. */
	#onLms(h: LmsHealth) {
		if (h.lms === 'busy') this.#markBusy(null, h.since);
		else this.#markOk();
	}

	/** LMS is busy: say so calmly (any error banner is explained by it) and poll less. */
	#markBusy(retryAfter: number | null, since: number | null = null) {
		this.#busy.markBusy(Date.now(), retryAfter, since);
		if (!this.serverBusy) {
			this.serverBusy = true;
			this.error = '';
			this.#reassess();
		}
	}

	/** LMS answers again: clear the status and catch up at once. */
	#markOk() {
		if (!this.#busy.markOk()) return;
		this.serverBusy = false;
		this.#reassess();
		if (this.notice === BUSY_MESSAGE) this.notice = '';
		if (this.status !== 'signed-in') return;
		void this.loadPlayers();
		this.soon();
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
			this.#failed(e);
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

	/** Signed out: forget which players were used here (and where this device was playing). */
	#forgetPlayers() {
		this.playerId = '';
		this.now = null;
		this.#preferred = '';
		this.wantDevice = false;
		saveTabSelection('');
		clearResume();
		try {
			localStorage.removeItem(PLAYER_KEY);
			localStorage.removeItem(REMOTE_KEY);
		} catch {
			/* storage blocked: nothing was kept */
		}
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
			this.notice = isBusy(e) ? BUSY_MESSAGE : `Couldn't move playback: ${(e as Error).message}`;
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

	/** Refresh the selected player's status; one request at a time (a call meanwhile asks again after it). */
	refresh(): Promise<void> {
		return this.#refresher.run();
	}

	async #refreshNow() {
		const id = this.playerId;
		if (!id || !this.current) return;
		try {
			const s = await api.rpc<PlayerStatus>(id, cmd.status());
			if (id !== this.playerId) return; // switched meanwhile
			this.now = s;
			this.#stale = false;
			upNextTracker.observe(id, stateOf(s));
			this.elapsed = typeof s.time === 'number' ? s.time : 0;
			this.#statusAt = performance.now();
			this.error = '';
			if (Date.now() - this.#rememberedAt > REMEMBER_MS) this.rememberNow();
			void this.#sleepCheck(id, s);
		} catch (e) {
			this.#failed(e);
		}
	}

	#advance() {
		const s = this.now;
		// The last state seen before the page loaded: where it is now isn't known.
		if (!s || this.#stale || s.mode !== 'play' || typeof s.time !== 'number') return;
		const t = s.time + (performance.now() - this.#statusAt) / 1000;
		this.elapsed = s.duration ? Math.min(t, s.duration) : t;
	}

	/**
	 * Send a command to the selected player, then refresh its status. Not while the server
	 * is out of reach or busy: the controls are disabled then, and the connection message
	 * says why, so a command that gets here anyway is dropped quietly (never replayed).
	 */
	async send(c: (string | number)[]) {
		const id = this.playerId;
		if (!id || !this.current || this.reach !== 'online') return;
		try {
			await api.rpc(id, c);
		} catch (e) {
			// Refused while LMS is busy, or the server couldn't be reached: it didn't happen,
			// isn't replayed later, and the connection message already says why.
			this.#failed(e);
		}
		this.soon();
	}

	/**
	 * Show what this device plays while the server is out of reach (see player/local):
	 * merged into the status, with the play head counted on from `time`.
	 */
	applyLocal(patch: PlayerStatus) {
		if (!this.now) return;
		this.now = { ...this.now, ...patch };
		if (typeof patch.time === 'number') {
			this.elapsed = patch.time;
			this.#statusAt = performance.now();
		}
	}

	/** Play, play next or add a selection on the selected player. */
	async queueUp(sel: cmd.Selection, placement: cmd.Placement, playIndex?: number) {
		if (!this.playerId) {
			this.error = NO_PLAYER;
			return;
		}
		if (this.reach !== 'online') return; // needs the server (the connection message says so)
		// Play next from this device: the queue's Up next section follows it.
		if (placement === 'next') upNextTracker.expectInsert(this.playerId, stateOf(this.now));
		await this.send(cmd.enqueue(sel, placement, playIndex));
		this.queueVersion++;
	}

	/** Play a selection in shuffled order (a page header's Shuffle): shuffle on, then load it. */
	async shufflePlay(sel: cmd.Selection) {
		if (!this.playerId) {
			this.error = NO_PLAYER;
			return;
		}
		if (this.reach !== 'online') return; // needs the server (the connection message says so)
		// With shuffle on, LMS shuffles what it loads.
		await this.send(cmd.shuffle(1));
		await this.send(cmd.enqueue(sel, 'play'));
		this.queueVersion++;
	}

	// --- sleep timer (docs/plans: Wave 1 §1) -------------------------------------------------

	/** The queue after the current track (durations and albums), read once per queue state. */
	async #readUpcoming(id: string, s: PlayerStatus): Promise<Track[]> {
		const cur = Number(s.playlist_cur_index ?? 0);
		const key = `${id}|${cur}|${s.playlist_tracks ?? 0}|${this.queueVersion}`;
		if (this.#upcoming?.key === key) return this.#upcoming.tracks;
		const r = await api.rpc<PlayerStatus>(id, cmd.queueTimes(cur + 1));
		const tracks = r.playlist_loop ?? [];
		this.#upcoming = { key, tracks };
		return tracks;
	}

	/**
	 * Set the sleep timer on the selected player, its sync group or every player. Resolves to
	 * an error message, or '' when set.
	 */
	async setSleep(choice: SleepChoice, scope: SleepScope = 'player'): Promise<string> {
		const id = this.playerId;
		const s = this.now;
		if (!id || !this.current) return NO_PLAYER;
		try {
			const upcoming = choice.kind === 'album' || choice.kind === 'queue' ? await this.#readUpcoming(id, s ?? {}) : [];
			const secs = choiceSeconds(choice, s, upcoming);
			if (secs === null || secs <= 0) return "Can't tell when that ends (a stream with no set length?).";
			const players = sleepTargets(scope, id, s, this.players);
			await Promise.all(players.map((p) => api.rpc(p, cmd.sleep(secs))));
			const now = Date.now();
			this.armed = choice.kind === 'minutes' ? null : { end: choice.kind, players, endsAt: now + secs * 1000, setAt: now };
			if (players.includes(this.deviceMac)) this.onDeviceSleep(now + secs * 1000, this.sleepFade);
			this.soon();
			return '';
		} catch (e) {
			return `Couldn't set the sleep timer: ${(e as Error).message}`;
		}
	}

	/** Turn the timer off: on the players it was set on, and on the selected one. */
	async cancelSleep(): Promise<string> {
		const players = [...new Set([...(this.armed?.players ?? []), this.playerId].filter(Boolean))];
		this.armed = null;
		if (players.includes(this.deviceMac)) this.onDeviceSleep(null, 0);
		try {
			await Promise.all(players.map((p) => api.rpc(p, cmd.sleep(0))));
			this.soon();
			return '';
		} catch (e) {
			return `Couldn't turn the sleep timer off: ${(e as Error).message}`;
		} finally {
			this.soon();
		}
	}

	/** Change the fade-out length (seconds). */
	setSleepFade(seconds: number) {
		saveFade(this.session?.user?.username ?? '', seconds);
		this.sleepFade = loadFade(this.session?.user?.username ?? '');
	}

	/**
	 * After each status: follow the timer on this device (its fade), and keep a timer set to
	 * an end of the queue in step with it (a seek, skip, pause or queue edit moves the end).
	 */
	async #sleepCheck(id: string, s: PlayerStatus) {
		const left = sleepLeft(s);
		if (id === this.deviceMac) this.onDeviceSleep(left === null ? null : Date.now() + left * 1000, this.sleepFade);
		const armed = this.armed;
		if (!armed || armed.players[0] !== id || this.#sleepBusy) return;
		const now = Date.now();
		if (armedOver(armed, s.will_sleep_in, now)) {
			this.armed = null;
			return;
		}
		this.#sleepBusy = true;
		try {
			const upcoming = armed.end === 'track' ? [] : await this.#readUpcoming(id, s);
			if (this.armed !== armed) return;
			const secs = rearmSeconds(armed, remainingSeconds(armed.end, s, upcoming), now);
			if (secs === null) return;
			await Promise.all(armed.players.map((p) => api.rpc(p, cmd.sleep(secs))));
			if (this.armed === armed) this.armed = { ...armed, endsAt: now + secs * 1000, setAt: now };
			if (armed.players.includes(this.deviceMac)) this.onDeviceSleep(now + secs * 1000, this.sleepFade);
		} catch {
			/* tried again at the next status */
		} finally {
			this.#sleepBusy = false;
		}
	}

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
