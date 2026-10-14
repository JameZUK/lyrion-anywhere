// LMS busy handling (docs/PROTOCOL.md): while the music server isn't answering, the gateway
// refuses commands at once (503 with `busy: true`) and says so on /v1/events. The app shows
// a calm status, keeps playing, and polls less often until LMS answers again.

/** The status poll's normal gap. */
export const POLL_MS = 2000;
/** The longest gap between status polls while LMS is busy. */
export const BUSY_POLL_MAX_MS = 16_000;

/**
 * Whether LMS is busy, and when the status poll may next ask. Pure (times are passed in), so
 * it is tested without timers.
 */
export class BusyGate {
	busy = false;
	/** When LMS stopped answering (unix ms, from the gateway), if known. */
	since: number | null = null;
	#refusals = 0;
	#nextPoll = 0;

	/**
	 * LMS is busy: from the events stream (no `retryAfter`) or a refused command. Each refusal
	 * in a row doubles the poll gap (from 2 s, up to 16 s), never shorter than Retry-After.
	 * Returns true if this is news.
	 */
	markBusy(now: number, retryAfterS: number | null = null, since: number | null = null): boolean {
		const news = !this.busy;
		this.busy = true;
		if (since !== null) this.since = since;
		else if (news) this.since = null;
		const backoff = Math.min(POLL_MS * 2 ** this.#refusals, BUSY_POLL_MAX_MS);
		if (retryAfterS !== null) this.#refusals++;
		const wait = Math.max(backoff, (retryAfterS ?? 0) * 1000);
		this.#nextPoll = Math.max(this.#nextPoll, now + Math.min(wait, BUSY_POLL_MAX_MS));
		return news;
	}

	/** LMS answers again. Returns true if it had been busy (refresh everything at once). */
	markOk(): boolean {
		const was = this.busy;
		this.busy = false;
		this.since = null;
		this.#refusals = 0;
		this.#nextPoll = 0;
		return was;
	}

	/** May the regular status poll run now? Always, unless LMS is busy and it isn't time yet. */
	mayPoll(now: number): boolean {
		return !this.busy || now >= this.#nextPoll;
	}
}

/**
 * Runs one task at a time: a call while one is running doesn't start another, but asks for
 * one more run once it finishes (an event may have changed things after the request went).
 * `again` decides whether that follow-up still runs (e.g. not while LMS is busy).
 */
export class Single {
	#running: Promise<void> | null = null;
	#again = false;

	constructor(
		private task: () => Promise<void>,
		private again: () => boolean = () => true,
	) {}

	get running(): boolean {
		return this.#running !== null;
	}

	/** Run now, or once more after the current run. Resolves when this request is served. */
	run(): Promise<void> {
		if (this.#running) {
			this.#again = true;
			return this.#running;
		}
		const p = (async () => {
			try {
				do {
					this.#again = false;
					try {
						await this.task();
					} catch {
						/* the task reports its own failures */
					}
				} while (this.#again && this.again());
			} finally {
				this.#again = false;
				this.#running = null;
			}
		})();
		this.#running = p;
		return p;
	}

	/** Run only if nothing is running (the regular poll: a request in flight is enough). */
	poll(): Promise<void> | null {
		return this.#running ? null : this.run();
	}
}
