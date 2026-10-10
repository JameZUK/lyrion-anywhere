// Keeping "this device" playing when the page is in the background (iOS especially).
//
// iOS suspends a page's Web Audio when it is backgrounded or the screen locks, unless the page
// has an actively playing media element. So while this device plays, a silent looping <audio>
// element runs alongside the AudioWorklet. It must be started inside the user's tap.
// The page lifecycle is logged and kept across reloads, so a reload (iOS evicting the tab) can
// be seen afterwards, and the app can offer to resume.

export interface MediaLike {
	loop: boolean;
	preload: string;
	play(): Promise<void>;
	pause(): void;
	setAttribute(name: string, value: string): void;
	readonly paused: boolean;
}

/** A silent looping media element that holds the browser's media session. */
export class KeepAlive {
	#el: MediaLike | null = null;

	constructor(
		private src: string,
		private make: (src: string) => MediaLike = (s) => new Audio(s),
	) {}

	/** Start (call from a user gesture). Resolves false if the browser refused. */
	async start(): Promise<boolean> {
		if (!this.#el) {
			const el = this.make(this.src);
			el.loop = true;
			el.preload = 'auto';
			el.setAttribute('playsinline', '');
			el.setAttribute('aria-hidden', 'true');
			this.#el = el;
		}
		try {
			await this.#el.play();
			return true;
		} catch {
			return false;
		}
	}

	/** Restart after the system paused it (e.g. a phone call). Not a user gesture: may fail. */
	async nudge(): Promise<void> {
		if (this.#el?.paused) await this.#el.play().catch(() => {});
	}

	stop() {
		this.#el?.pause();
		this.#el = null;
	}

	get running(): boolean {
		return !!this.#el && !this.#el.paused;
	}
}

type Store = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;
const LOG_KEY = 'anywhere.device.log';
const ACTIVE_KEY = 'anywhere.device.active';
const MAX_LINES = 200;

/** The connection log, kept in session storage so it survives a reload of this tab. */
export class PersistentLog {
	lines: string[];

	constructor(private store: Store | undefined = safeSession()) {
		this.lines = this.#load();
	}

	#load(): string[] {
		try {
			const v = JSON.parse(this.store?.getItem(LOG_KEY) ?? '[]');
			return Array.isArray(v) ? v.filter((x) => typeof x === 'string').slice(0, MAX_LINES) : [];
		} catch {
			return [];
		}
	}

	add(msg: string, now = new Date()): string[] {
		const line = `${now.toLocaleTimeString('en-GB')}  ${msg.slice(0, 300)}`;
		this.lines = [line, ...this.lines].slice(0, MAX_LINES);
		try {
			this.store?.setItem(LOG_KEY, JSON.stringify(this.lines));
		} catch {
			/* full or blocked: the in-memory log still works */
		}
		return this.lines;
	}

	/** Remember whether this device was playing, to offer "resume" after a reload. */
	set active(on: boolean) {
		try {
			if (on) this.store?.setItem(ACTIVE_KEY, '1');
			else this.store?.removeItem(ACTIVE_KEY);
		} catch {
			/* not essential */
		}
	}

	get active(): boolean {
		try {
			return this.store?.getItem(ACTIVE_KEY) === '1';
		} catch {
			return false;
		}
	}
}

function safeSession(): Store | undefined {
	try {
		return globalThis.sessionStorage;
	} catch {
		return undefined;
	}
}

/** Short descriptions of page lifecycle events, for the log. */
export function describeLifecycle(type: string, detail: { visibility?: string; persisted?: boolean } = {}): string {
	switch (type) {
		case 'visibilitychange':
			return detail.visibility === 'hidden' ? 'page hidden (app switched or screen locked)' : 'page visible again';
		case 'pagehide':
			return detail.persisted ? 'page suspended' : 'page closing';
		case 'pageshow':
			return detail.persisted ? 'page restored from suspension' : 'page loaded';
		case 'freeze':
			return 'page frozen by the browser';
		case 'resume':
			return 'page unfrozen';
		default:
			return type;
	}
}
