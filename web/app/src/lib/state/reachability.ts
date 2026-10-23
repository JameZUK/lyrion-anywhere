// Can the app reach its server right now? One answer for every control surface: `online`,
// `busy` (the gateway answers but LMS is frozen) or `unreachable` (no network, or the
// gateway or this device's player socket has been down for a few seconds).
//
// Pure (times are passed in), so it is tested without timers. The app feeds it what it sees
// (the events socket, failed requests, the browser's own online flag, this device's player
// socket) and re-asks at `recheckAt` so a brief blip never flashes the UI.
import { ApiError } from '#lib/api/http';
import type { EventsState } from '#lib/api/events';

export type Reachability = 'online' | 'busy' | 'unreachable';

/** How long a connection has to stay down before the app says so (and greys controls out). */
export const UNREACHABLE_AFTER_MS = 3000;

/**
 * Did a request fail because the server couldn't be reached at all (rather than refusing
 * it)? `fetch` rejects with a TypeError when there is no network or the connection breaks;
 * a proxy in front of a gateway that is down answers 502, 504 or a 503 that isn't the
 * gateway's own busy answer.
 */
export function isNetworkError(e: unknown): boolean {
	if (e instanceof TypeError) return true;
	if (e instanceof ApiError) return e.status === 0 || e.status === 502 || e.status === 504 || (e.status === 503 && !e.busy);
	return false;
}

export interface Signals {
	/** `navigator.onLine` (false means certainly offline; true proves nothing). */
	browserOnline: boolean;
	/** The events socket. */
	events: EventsState;
	/** The last request failed for want of a connection (cleared by the next success). */
	requestsFailing: boolean;
	/** This device is the player and its socket to the gateway is down. */
	deviceDown: boolean;
	/** The gateway says LMS isn't answering. */
	busy: boolean;
}

export class ReachabilityTracker {
	#downSince: number | null = null;
	#wasLive = false;

	/**
	 * The state now, and when to ask again (a connection that is down but not yet for long
	 * enough becomes `unreachable` then), or null when nothing is pending.
	 */
	update(s: Signals, now: number): { state: Reachability; recheckAt: number | null } {
		if (s.events === 'live') this.#wasLive = true;
		if (s.events === 'stopped') this.#wasLive = false;
		// The events socket's first connection isn't a lost one: only a socket that was live
		// and is now trying again says the server has gone.
		const socketDown = s.events === 'retrying' || (s.events === 'connecting' && this.#wasLive);
		const down = !s.browserOnline || socketDown || s.requestsFailing || s.deviceDown;
		if (!down) {
			this.#downSince = null;
			return { state: s.busy ? 'busy' : 'online', recheckAt: null };
		}
		this.#downSince ??= now;
		// The browser saying it is offline is certain: no need to wait.
		if (!s.browserOnline || now - this.#downSince >= UNREACHABLE_AFTER_MS) return { state: 'unreachable', recheckAt: null };
		return { state: s.busy ? 'busy' : 'online', recheckAt: this.#downSince + UNREACHABLE_AFTER_MS };
	}

	reset() {
		this.#downSince = null;
		this.#wasLive = false;
	}
}

export const BUSY_TEXT = 'Your music server is busy. Waiting for it…';
export const UNREACHABLE_TEXT = 'Can’t reach your music server.';
export const LOCAL_TEXT = 'Can’t reach your music server. Playing from this device.';

/** The connection message's element id, for the controls it explains. */
export const CONNECTION_ID = 'connection-status';

/** What a disabled control says about why (nothing when the connection isn't the reason). */
export interface Explained {
	title?: string;
	'aria-describedby'?: string;
}

/** Point a control disabled by the connection at the connection message. */
export function explain(disabled: boolean, state: Reachability, message: string): Explained {
	return disabled && state !== 'online' && message ? { title: message, 'aria-describedby': CONNECTION_ID } : {};
}

/**
 * The attributes of a control that needs the server: disabled unless it is in reach and `ok`
 * (anything else the control needs), and saying why when the connection is the reason.
 */
export function needsServer(state: Reachability, message: string, ok = true): Explained & { disabled: boolean } {
	const disabled = state !== 'online' || !ok;
	return { disabled, ...explain(disabled, state, message) };
}

/**
 * The one connection message: '' while online. `playingHere`: this device plays on by
 * itself; `waiting`: why it stopped (e.g. the next track needs the server), if it did.
 */
export function connectionMessage(state: Reachability, playingHere: boolean, waiting = ''): string {
	if (state === 'online') return '';
	const base = state === 'busy' ? BUSY_TEXT : playingHere ? LOCAL_TEXT : UNREACHABLE_TEXT;
	return waiting ? `${base} ${waiting}` : base;
}
