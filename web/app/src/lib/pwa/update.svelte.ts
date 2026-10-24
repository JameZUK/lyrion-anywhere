// A new version of the app applies itself. The service worker installs a new version and
// waits (see sw-handlers); this page then lets it take over and reloads at once, unless this
// device is playing (a reload stops this device's playback). While it plays, the status slot
// shows a quiet "A new version is ready" (Reload / Later), and the new version applies itself
// the moment playback here stops. "Later" holds it until the next new version or the next
// launch. See `decideUpdate` for the whole rule.
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

/** How often the app asks the server for a new version while it stays open (and, at most, on coming back to it). */
export const CHECK_EVERY_MS = 60 * 60 * 1000;

export type UpdateAction = 'none' | 'apply' | 'offer' | 'hold';

/**
 * What to do about a new version:
 * - `none`: nothing waiting, or a first install (no controller: nothing to upgrade from);
 * - `hold`: the person said "Later" (until the next new version or the next launch);
 * - `offer`: this device is playing: show the offer, apply when playback here stops;
 * - `apply`: take over and reload now.
 */
export function decideUpdate(s: { waiting: boolean; controlled: boolean; playing: boolean; dismissed: boolean }): UpdateAction {
	if (!s.waiting || !s.controlled) return 'none';
	if (s.dismissed) return 'hold';
	return s.playing ? 'offer' : 'apply';
}

/**
 * Whether a reload would interrupt playback on this device: its downloads player is in use,
 * or its own player is on and either sounding or holding a track (playing or paused) as the
 * player being controlled. A player that is off, stopped or failed interrupts nothing.
 */
export function playingHere(s: {
	/** The downloads engine is in use (playing or paused). */
	downloads: boolean;
	/** This device's player (`device.state`). */
	deviceState: string;
	/** This device's player is the one being controlled. */
	thisDevice: boolean;
	/** LMS's mode for the controlled player (`play`, `pause`, `stop`), if known. */
	mode: string | undefined;
	/** This device's own output state (`device.detail.output`). */
	output: string | undefined;
}): boolean {
	if (s.downloads) return true;
	if (s.deviceState !== 'starting' && s.deviceState !== 'connected' && s.deviceState !== 'reconnecting') return false;
	if (s.output === 'Playing') return true;
	return s.thisDevice && (s.mode === 'play' || s.mode === 'pause');
}

export interface UpdateOptions {
	container?: SwContainer | null;
	reload?: () => void;
	now?: () => number;
	/** Starts the hourly check; returns the way to stop it (tests pass a fake). */
	every?: (fn: () => void, ms: number) => () => void;
}

const realEvery = (fn: () => void, ms: number) => {
	const t = setInterval(fn, ms);
	return () => clearInterval(t);
};

export class AppUpdate {
	/** A new version is installed and waiting. */
	ready = $state(false);
	/** The person said "Later" (until the next new version, or the next launch). */
	dismissed = $state(false);
	/** This device is playing (see `playingHere`), so the new version waits for it to stop. */
	playing = $state(false);
	#container: SwContainer | null = null;
	#reg: SwRegistration | null = null;
	#reload: () => void = () => globalThis.location?.reload();
	#lastCheck = 0;
	#now: () => number = () => Date.now();
	#every: (fn: () => void, ms: number) => () => void = realEvery;
	#stopTimer: (() => void) | null = null;
	#applying = false;

	/** Whether to show the offer. */
	get offered(): boolean {
		return this.#action() === 'offer';
	}

	#action(): UpdateAction {
		return decideUpdate({ waiting: this.ready, controlled: !!this.#container?.controller, playing: this.playing, dismissed: this.dismissed });
	}

	/**
	 * Start watching. `version` is the build this page runs (SvelteKit's `version`, the same as
	 * the service worker's). Tests pass a fake container, reload, clock and timer.
	 */
	start(version: string, o: UpdateOptions = {}) {
		const container = o.container !== undefined ? o.container : ((globalThis.navigator?.serviceWorker as unknown as SwContainer | undefined) ?? null);
		if (!container || this.#container) return;
		this.#container = container;
		if (o.reload) this.#reload = o.reload;
		if (o.now) this.#now = o.now;
		if (o.every) this.#every = o.every;
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
		// A version already waiting at launch applies before anything plays.
		if (reg.waiting) this.#found();
		reg.addEventListener('updatefound', () => {
			const w = reg.installing;
			if (!w) return;
			w.addEventListener('statechange', () => {
				if (w.state === 'installed') this.#found();
			});
		});
		// Every launch asks for a new version, then every hour while the app stays open.
		this.#update();
		this.#stopTimer = this.#every(() => this.#update(), CHECK_EVERY_MS);
	}

	#update() {
		this.#lastCheck = this.#now();
		void this.#reg?.update().catch(() => {});
	}

	#found() {
		this.ready = true;
		this.dismissed = false;
		this.#decide();
	}

	#decide() {
		if (this.#action() === 'apply') this.apply();
	}

	/** Tell whether this device is playing; stopping lets a waiting new version apply itself. */
	setPlaying(playing: boolean) {
		if (playing === this.playing) return;
		this.playing = playing;
		this.#decide();
	}

	/** Coming back to the app: look for a new version if the last look was an hour ago or more. */
	check() {
		if (!this.#reg || this.#now() - this.#lastCheck < CHECK_EVERY_MS) return;
		this.#update();
	}

	/** Let the new version take over, then reload once it controls this page ("Reload", or by itself). */
	apply() {
		if (this.#applying) return;
		const waiting = this.#reg?.waiting;
		this.#applying = true;
		if (!waiting || !this.#container) {
			this.#reload();
			return;
		}
		let done = false;
		const reload = () => {
			if (done) return;
			done = true;
			this.#reload();
		};
		this.#container.addEventListener('controllerchange', reload, { once: true });
		// Some browsers are slow to say the controller changed: the new worker active is enough.
		waiting.addEventListener('statechange', () => waiting.state === 'activated' && reload());
		waiting.postMessage({ type: MSG.skipWaiting });
	}

	/** "Later": hold the new version until the next one, or the next launch. */
	dismiss() {
		this.dismissed = true;
	}

	/** Stop the hourly check (tests). */
	stop() {
		this.#stopTimer?.();
		this.#stopTimer = null;
	}
}

export const appUpdate = new AppUpdate();
