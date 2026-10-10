// "This device": the browser as an LMS player. The WASM core runs in a worker, which owns
// the gateway socket and stream downloads; audio plays through an AudioWorklet.
import { session } from '$lib/state/app.svelte';
import { KeepAlive, PersistentLog, describeLifecycle } from './background';

export type DeviceState = 'off' | 'starting' | 'connected' | 'reconnecting' | 'stopped' | 'error';

export interface DeviceStatus {
	output?: string;
	elapsed_ms?: number;
	buffered_ms?: number;
	sample_rate?: number;
	last_error?: string;
	input_bytes?: number;
	wasm_bytes?: number;
}

const asset = (p: string) => new URL(`player/${p}`, document.baseURI).toString();

class DevicePlayer {
	state = $state<DeviceState>('off');
	mac = $state('');
	name = $state('');
	detail = $state<DeviceStatus>({});
	#history = new PersistentLog();
	log = $state<string[]>(this.#history.lines);
	/** This tab was playing before a reload: offer to resume (audio needs a tap to start). */
	wasActive = $state(this.#history.active);
	/** Called when the player is connected (the app selects it). */
	onConnected: ((mac: string) => void) | null = null;

	#worker: Worker | null = null;
	#ctx: AudioContext | null = null;
	#unsubscribe: (() => void) | null = null;
	#detailAt = 0;
	#keepAlive = new KeepAlive(asset('silence.wav'));
	#lifecycleBound = false;

	#note(msg: string) {
		this.log = this.#history.add(msg);
	}

	/** Log page lifecycle changes, and recover audio when the page comes back. */
	#watchLifecycle() {
		if (this.#lifecycleBound) return;
		this.#lifecycleBound = true;
		const active = () => !!this.#worker;
		document.addEventListener('visibilitychange', () => {
			if (!active()) return;
			this.#note(describeLifecycle('visibilitychange', { visibility: document.visibilityState }));
			if (document.visibilityState === 'visible') void this.#recover();
		});
		for (const type of ['pagehide', 'pageshow'] as const) {
			addEventListener(type, (e) => {
				if (active() || type === 'pageshow') this.#note(describeLifecycle(type, { persisted: (e as PageTransitionEvent).persisted }));
				if (type === 'pageshow' && (e as PageTransitionEvent).persisted) void this.#recover();
			});
		}
		for (const type of ['freeze', 'resume'] as const) {
			document.addEventListener(type, () => active() && this.#note(describeLifecycle(type)));
		}
	}

	/** After the system suspended us: restart the keep-alive and the audio context. */
	async #recover() {
		await this.#keepAlive.nudge();
		const ctx = this.#ctx;
		if (ctx && ctx.state !== 'running' && ctx.state !== 'closed') {
			try {
				await ctx.resume();
				this.#note(`audio resumed (${ctx.state})`);
			} catch (e) {
				this.#note(`audio could not resume: ${(e as Error).message}; tap Play on this device`);
			}
		}
	}

	get supported(): boolean {
		return typeof AudioWorkletNode !== 'undefined' && typeof Worker !== 'undefined' && typeof WebAssembly !== 'undefined';
	}

	/** Start playing on this device. Must be called from a click (browsers require it). */
	async start() {
		if (this.#worker || !session.current) return;
		this.state = 'starting';
		this.wasActive = false;
		this.#watchLifecycle();
		try {
			this.#configureAudioSession();
			// Inside the tap: a playing media element keeps iOS from suspending the page.
			if (!(await this.#keepAlive.start())) this.#note('background keep-alive refused by the browser');
			const port = await this.#createAudio(44100);
			const w = new Worker(asset('player-worker.js'), { type: 'module' });
			this.#worker = w;
			w.onmessage = (e) => void this.#onMessage(e.data);
			w.onerror = (e) => {
				this.#note(`worker error: ${e.message}`);
				this.state = 'error';
			};
			w.postMessage({ cmd: 'start', token: session.token, port }, [port]);
			this.#history.active = true;
			w.postMessage({ cmd: 'rate-ready', rate: 44100 });
			// Pass renewed access tokens to the worker; stop if signed out.
			this.#unsubscribe = session.subscribe((s) => {
				if (!s) this.stop();
				else this.#worker?.postMessage({ cmd: 'token', token: s.access_token });
			});
		} catch (e) {
			this.#note(`could not start: ${(e as Error).message}`);
			this.state = 'error';
			this.stop('error');
		}
	}

	stop(final: DeviceState = 'stopped') {
		this.#history.active = false;
		this.wasActive = false;
		this.#keepAlive.stop();
		this.#unsubscribe?.();
		this.#unsubscribe = null;
		this.#worker?.terminate();
		this.#worker = null;
		void this.#ctx?.close();
		this.#ctx = null;
		this.state = final === 'error' ? 'error' : this.state === 'off' ? 'off' : 'stopped';
	}

	// iOS/Safari 16.4+: declare this page a media player, so audio keeps playing with the
	// screen locked and ignores the silent switch. A no-op elsewhere.
	#configureAudioSession() {
		const nav = navigator as Navigator & { audioSession?: { type: string } };
		if (nav.audioSession) {
			try {
				nav.audioSession.type = 'playback';
			} catch (e) {
				this.#note(`audio session not configurable: ${(e as Error).message}`);
			}
		}
	}

	async #createAudio(rate: number): Promise<MessagePort> {
		if (this.#ctx) await this.#ctx.close();
		// The core outputs at the track's own rate; the browser resamples to the hardware.
		const ctx = new AudioContext({ sampleRate: rate, latencyHint: 'playback' });
		this.#ctx = ctx;
		await ctx.audioWorklet.addModule(asset('worklet.js'));
		const node = new AudioWorkletNode(ctx, 'anywhere-sink', { outputChannelCount: [2] });
		node.connect(ctx.destination);
		const channel = new MessageChannel();
		node.port.postMessage({ port: channel.port1 }, [channel.port1]);
		await ctx.resume();
		return channel.port2;
	}

	async #onMessage(m: Record<string, unknown>) {
		if (typeof m.log === 'string') this.#note(m.log);
		if (m.connection === 'connected') {
			this.state = 'connected';
			const p = m.player as { mac?: string; name?: string } | undefined;
			if (p?.mac && p.mac !== this.mac) {
				this.mac = p.mac;
				this.name = p.name ?? '';
				this.onConnected?.(p.mac);
			}
		} else if (m.connection === 'reconnecting' || m.connection === 'restarting' || m.connection === 'reauthenticating') {
			this.state = 'reconnecting';
		} else if (m.connection === 'closed') {
			this.state = 'stopped';
		}
		if (m.authFailed) {
			// The access token was refused (expired): renew it and let the worker carry on.
			if (await session.renew()) this.#worker?.postMessage({ cmd: 'token', token: session.token, reconnect: true });
		}
		if (typeof m.rate === 'number' && this.#worker) {
			const port = await this.#createAudio(m.rate);
			this.#worker.postMessage({ cmd: 'start-sink', port }, [port]);
			this.#worker.postMessage({ cmd: 'rate-ready', rate: m.rate });
		}
		// The worker reports status every 20 ms; the UI only needs a few updates a second.
		if (m.status && typeof m.status === 'object' && performance.now() - this.#detailAt > 500) {
			this.#detailAt = performance.now();
			this.detail = m.status as DeviceStatus;
		}
	}
}

export const device = new DevicePlayer();
