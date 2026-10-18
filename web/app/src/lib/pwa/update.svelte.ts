// A new version of the app, offered rather than forced. The service worker installs a new
// version and waits (see sw-handlers); this page keeps running the one it loaded and shows a
// quiet "A new version is ready" in the status slot. Reloading is the person's choice, because
// a reload stops this device's playback.
import { MSG } from './sw-handlers';

/** The parts of `navigator.serviceWorker` used here. */
export interface SwContainer {
	readonly controller: { postMessage(m: unknown): void } | null;
	getRegistration(): Promise<SwRegistration | undefined>;
	addEventListener(type: 'message' | 'controllerchange', fn: (e: MessageEvent) => void, opts?: { once?: boolean }): void;
}
export interface SwWorker {
	readonly state: string;
	postMessage(m: unknown): void;
	addEventListener(type: 'statechange', fn: () => void): void;
}
export interface SwRegistration {
	readonly waiting: SwWorker | null;
	readonly installing: SwWorker | null;
	update(): Promise<unknown>;
	addEventListener(type: 'updatefound', fn: () => void): void;
}

/** How often, at most, coming back to the app asks the server for a new version. */
export const CHECK_EVERY_MS = 60 * 60 * 1000;

export class AppUpdate {
	/** A new version is installed and waiting. */
	ready = $state(false);
	/** The person said "Later" (until the next new version). */
	dismissed = $state(false);
	#container: SwContainer | null = null;
	#reg: SwRegistration | null = null;
	#reload: () => void = () => globalThis.location?.reload();
	#lastCheck = 0;
	#now: () => number = () => Date.now();

	/** Whether to show the offer. */
	get offered(): boolean {
		return this.ready && !this.dismissed;
	}

	/**
	 * Start watching. `version` is the build this page runs (SvelteKit's `version`, the same as
	 * the service worker's). Tests pass a fake container, reload and clock.
	 */
	start(version: string, o: { container?: SwContainer | null; reload?: () => void; now?: () => number } = {}) {
		const container = o.container !== undefined ? o.container : ((globalThis.navigator?.serviceWorker as unknown as SwContainer | undefined) ?? null);
		if (!container || this.#container) return;
		this.#container = container;
		if (o.reload) this.#reload = o.reload;
		if (o.now) this.#now = o.now;
		this.#lastCheck = this.#now();
		// The worker asks which version each open page runs before deleting a previous one.
		container.addEventListener('message', (e) => {
			const d = e.data as { type?: unknown } | null;
			if (d && typeof d === 'object' && d.type === MSG.ask && e.ports?.[0]) e.ports[0].postMessage({ type: MSG.reply, version });
		});
		try {
			container.controller?.postMessage({ type: MSG.hello });
		} catch {
			/* no worker to tell */
		}
		void container
			.getRegistration()
			.then((reg) => reg && this.#watch(reg))
			.catch(() => {});
	}

	#watch(reg: SwRegistration) {
		this.#reg = reg;
		const hasController = () => !!this.#container?.controller;
		// A first install (no controller yet) is not an upgrade: nothing to offer.
		if (reg.waiting && hasController()) this.#found();
		reg.addEventListener('updatefound', () => {
			const w = reg.installing;
			if (!w) return;
			w.addEventListener('statechange', () => {
				if (w.state === 'installed' && hasController()) this.#found();
			});
		});
	}

	#found() {
		this.ready = true;
		this.dismissed = false;
	}

	/** Coming back to the app: look for a new version, at most once an hour. */
	check() {
		if (!this.#reg || this.#now() - this.#lastCheck < CHECK_EVERY_MS) return;
		this.#lastCheck = this.#now();
		void this.#reg.update().catch(() => {});
	}

	/** "Reload": let the new version take over, then reload once it controls this page. */
	apply() {
		const waiting = this.#reg?.waiting;
		if (!waiting || !this.#container) {
			this.#reload();
			return;
		}
		this.#container.addEventListener('controllerchange', () => this.#reload(), { once: true });
		waiting.postMessage({ type: MSG.skipWaiting });
	}

	/** "Later". */
	dismiss() {
		this.dismissed = true;
	}
}

export const appUpdate = new AppUpdate();
