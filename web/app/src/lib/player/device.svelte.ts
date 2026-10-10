// "This device": the browser as an LMS player. The WASM core runs in a worker, which owns
// the gateway socket and stream downloads; audio plays through an AudioWorklet.
import { session } from '$lib/state/app.svelte';

export type DeviceState = 'off' | 'starting' | 'connected' | 'reconnecting' | 'stopped' | 'error';

export interface DeviceStatus {
	output?: string;
	elapsed_ms?: number;
	buffered_ms?: number;
	sample_rate?: number;
	last_error?: string;
}

const asset = (p: string) => new URL(`player/${p}`, document.baseURI).toString();

class DevicePlayer {
	state = $state<DeviceState>('off');
	mac = $state('');
	name = $state('');
	detail = $state<DeviceStatus>({});
	log = $state<string[]>([]);
	/** Called when the player is connected (the app selects it). */
	onConnected: ((mac: string) => void) | null = null;

	#worker: Worker | null = null;
	#ctx: AudioContext | null = null;
	#unsubscribe: (() => void) | null = null;
	#detailAt = 0;

	#note(msg: string) {
		const line = `${new Date().toLocaleTimeString('en-GB')}  ${msg}`;
		this.log = [line, ...this.log].slice(0, 100);
	}

	get supported(): boolean {
		return typeof AudioWorkletNode !== 'undefined' && typeof Worker !== 'undefined' && typeof WebAssembly !== 'undefined';
	}

	/** Start playing on this device. Must be called from a click (browsers require it). */
	async start() {
		if (this.#worker || !session.current) return;
		this.state = 'starting';
		try {
			this.#configureAudioSession();
			const port = await this.#createAudio(44100);
			const w = new Worker(asset('player-worker.js'), { type: 'module' });
			this.#worker = w;
			w.onmessage = (e) => void this.#onMessage(e.data);
			w.onerror = (e) => {
				this.#note(`worker error: ${e.message}`);
				this.state = 'error';
			};
			w.postMessage({ cmd: 'start', token: session.token, port }, [port]);
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
