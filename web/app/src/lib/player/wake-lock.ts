// Keeping the screen on (car mode, wave2-design.md §5.1) through the Screen Wake Lock API,
// where the browser has it. The lock is held only while wanted (playing) and the page is
// visible: browsers drop it when the page is hidden, so it is asked for again on return, and
// it is released when no longer wanted or when the screen that asked goes away.

export interface WakeSentinel {
	readonly released: boolean;
	release(): Promise<void>;
	addEventListener?(type: 'release', fn: () => void): void;
}
export interface WakeLockApi {
	request(type: 'screen'): Promise<WakeSentinel>;
}

function systemApi(): WakeLockApi | null {
	const n = typeof navigator !== 'undefined' ? (navigator as Navigator & { wakeLock?: WakeLockApi }) : null;
	return n?.wakeLock && typeof n.wakeLock.request === 'function' ? n.wakeLock : null;
}

export class ScreenWake {
	#api: WakeLockApi | null;
	#doc: Document | null;
	#sentinel: WakeSentinel | null = null;
	#want = false;
	#asking = false;
	#closed = false;
	#onVisibility = () => void this.#sync();

	constructor(api: WakeLockApi | null = systemApi(), doc: Document | null = typeof document !== 'undefined' ? document : null) {
		this.#api = api;
		this.#doc = doc;
		doc?.addEventListener('visibilitychange', this.#onVisibility);
	}

	/** Whether this browser can keep the screen on. */
	get supported(): boolean {
		return this.#api !== null;
	}

	/** Whether the screen is being kept on now. */
	get held(): boolean {
		return !!this.#sentinel && !this.#sentinel.released;
	}

	/** Keep the screen on (while visible), or let it go. */
	set(want: boolean) {
		if (this.#closed) return;
		this.#want = want;
		void this.#sync();
	}

	/** Let go for good (the screen that asked has gone). */
	destroy() {
		this.#closed = true;
		this.#want = false;
		this.#doc?.removeEventListener('visibilitychange', this.#onVisibility);
		void this.#release();
	}

	async #release() {
		const s = this.#sentinel;
		this.#sentinel = null;
		if (s && !s.released) {
			try {
				await s.release();
			} catch {
				/* already gone */
			}
		}
	}

	async #sync() {
		const visible = !this.#doc || this.#doc.visibilityState !== 'hidden';
		if (!this.#want || !visible || this.#closed) {
			await this.#release();
			return;
		}
		if (!this.#api || this.held || this.#asking) return;
		this.#asking = true;
		try {
			const s = await this.#api.request('screen');
			// Still wanted? (Paused, hidden or left while asking: give it straight back.)
			if (!this.#want || this.#closed || this.#doc?.visibilityState === 'hidden') {
				await s.release().catch(() => {});
				return;
			}
			this.#sentinel = s;
			s.addEventListener?.('release', () => {
				if (this.#sentinel === s) this.#sentinel = null;
			});
		} catch {
			/* refused (e.g. battery saver): the screen may sleep */
		} finally {
			this.#asking = false;
		}
	}
}
