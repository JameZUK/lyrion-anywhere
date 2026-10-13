// The events stream (WS /v1/events): player and library change notices, used as hints to
// re-query state. Reconnects with backoff; renews the token when the gateway refuses it.
import type { SessionManager } from './session';

export interface LmsEvent {
	player: string | null;
	event: string[];
}

export type EventsState = 'connecting' | 'live' | 'retrying' | 'stopped';

export interface EventsHandlers {
	onEvent: (e: LmsEvent) => void;
	/** Some events were missed (or the socket reconnected): re-query everything. */
	onResync: () => void;
	onState?: (s: EventsState) => void;
}

/** Backoff with jitter: ~1 s, 2 s, 4 s … up to ~30 s. */
export function eventsBackoff(attempt: number, random = Math.random): number {
	return Math.round(Math.min(1000 * 2 ** attempt, 30_000) * (0.8 + 0.4 * random()));
}

/** Parse one message from the gateway; anything unexpected is ignored. */
export function parseEventMessage(text: unknown): LmsEvent | 'resync' | 'subscribed' | null {
	if (typeof text !== 'string' || text.length > 65_536) return null;
	let m: unknown;
	try {
		m = JSON.parse(text);
	} catch {
		return null;
	}
	if (!m || typeof m !== 'object') return null;
	const o = m as Record<string, unknown>;
	if (o.subscribed === true) return 'subscribed';
	if (o.resync === true) return 'resync';
	if (!Array.isArray(o.event) || !o.event.every((t) => typeof t === 'string')) return null;
	if (o.player !== null && typeof o.player !== 'string') return null;
	return { player: o.player as string | null, event: o.event as string[] };
}

export class EventsClient {
	#ws: WebSocket | null = null;
	#attempt = 0;
	#stopped = false;
	#timer: ReturnType<typeof setTimeout> | undefined;

	constructor(
		private session: SessionManager,
		private h: EventsHandlers,
		private makeSocket: (url: string) => WebSocket = (u) => new WebSocket(u),
	) {}

	start() {
		this.#stopped = false;
		this.#connect();
	}

	stop() {
		this.#stopped = true;
		clearTimeout(this.#timer);
		this.#ws?.close(1000);
		this.#ws = null;
		this.h.onState?.('stopped');
	}

	#connect() {
		if (this.#stopped || !this.session.current) return;
		this.h.onState?.('connecting');
		const url = new URL('/v1/events', location.href);
		url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
		const ws = this.makeSocket(url.toString());
		this.#ws = ws;
		ws.onopen = () => ws.send(this.session.token);
		ws.onmessage = (m) => {
			const msg = parseEventMessage(m.data);
			if (msg === 'subscribed') {
				this.#attempt = 0;
				this.h.onState?.('live');
				this.h.onResync(); // anything may have changed while we were away
			} else if (msg === 'resync') this.h.onResync();
			else if (msg) this.h.onEvent(msg);
		};
		ws.onclose = async (e) => {
			if (this.#ws !== ws || this.#stopped) return;
			this.#ws = null;
			if (e.code === 1008 && e.reason === 'unauthorised') {
				// Probably an expired token: renew, then reconnect. A renewal that can't reach
				// the server is retried by the session; keep trying here too (with backoff)
				// unless it actually signed the user out.
				const renewed = await this.session.renew('events');
				if (this.#ws !== null || this.#stopped) return; // restarted or stopped meanwhile
				if (!renewed && !this.session.current) return this.stop();
			} else if (e.code === 1008) {
				return this.stop(); // revoked: the session listener signs out
			}
			this.h.onState?.('retrying');
			this.#timer = setTimeout(() => this.#connect(), eventsBackoff(this.#attempt++));
		};
	}
}
