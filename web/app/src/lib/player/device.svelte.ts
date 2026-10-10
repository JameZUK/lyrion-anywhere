// "This device": the browser as an LMS player. The WASM core runs in a worker, which owns
// the gateway socket and stream downloads. Audio plays through an AudioWorklet, or through an
// <audio> element (Media Source) where only that keeps playing in the background (iOS).
import { session } from '$lib/state/app.svelte';
import { KeepAlive, PersistentLog, describeLifecycle } from './background';
import { detect } from './capabilities';
import { MediaOutput, Timeline } from './media-output';
import { MODE_LABELS, chooseMode, detectEnvironment, loadPreference, type ModeChoice } from './mode';
import { Reporter, buildReport } from './diagnostics';
import { chooseVariant, loadCached, mediaFormatFor, runSelfTest, saveCached, type SelfTest } from './selftest';
import { CLIENT_HEADER, bearer } from '$lib/api/http';

export type DeviceState = 'off' | 'starting' | 'connected' | 'reconnecting' | 'stopped' | 'error';

export interface DeviceStatus {
	output?: string;
	elapsed_ms?: number;
	buffered_ms?: number;
	sample_rate?: number;
	last_error?: string;
	input_bytes?: number;
	wasm_bytes?: number;
	mode?: string;
}

const asset = (p: string) => new URL(`player/${p}`, document.baseURI).toString();

class DevicePlayer {
	state = $state<DeviceState>('off');
	mac = $state('');
	name = $state('');
	detail = $state<DeviceStatus>({});
	#history = new PersistentLog();
	log = $state<string[]>(this.#history.lines);
	/** This tab was playing before a reload: it resumes by itself (see `resumeAfterReload`). */
	wasActive = $state(this.#history.active);
	/** Playing, but the browser holds the sound back until the user taps the page. */
	needsTap = $state(false);
	/** How this device plays, and why (chosen at start). */
	choice = $state<ModeChoice | null>(null);
	/** Which media-element variants this browser really plays (see selftest.ts). */
	selftest = $state<SelfTest | null>(null);
	#selftestRunning: Promise<void> | null = null;
	/** The media element failed in this session: use Web Audio until the page reloads. */
	#mediaBroken = false;
	/** Started without a tap (after a reload, or switching output). */
	#auto = false;
	/** Called when the player is connected (the app selects it). */
	onConnected: ((mac: string) => void) | null = null;

	#worker: Worker | null = null;
	#ctx: AudioContext | null = null;
	#unsubscribe: (() => void) | null = null;
	#detailAt = 0;
	#keepAlive = new KeepAlive(asset('silence.wav'));
	#lifecycleBound = false;
	#media: MediaOutput | null = null;
	#timeline = new Timeline();
	#lastPos = 0;
	#posTimer: ReturnType<typeof setInterval> | undefined;
	#advancedAt = 0;
	#drainedSent = false;
	#drainedAtEnd = 0; // buffered end when "drained" was last sent
	/** Diagnostics for the admin: batched, and sent at once when the page is hidden. */
	#reporter = new Reporter(
		(body, urgent) =>
			session.authed(async (token) => {
				await fetch('/v1/diagnostics', {
					method: 'POST',
					headers: { ...CLIENT_HEADER, ...bearer(token), 'Content-Type': 'application/json' },
					body,
					keepalive: urgent,
				});
			}),
		() => this.#diagnostics(),
	);

	#note(msg: string) {
		this.log = this.#history.add(msg);
		if (session.current) this.#reporter.schedule();
	}

	#diagnostics() {
		const caps = detect();
		const environment = detectEnvironment();
		const preference = loadPreference();
		return buildReport({
			capabilities: caps,
			environment,
			preference,
			planned: chooseMode(caps, environment, preference),
			active: this.#worker ? this.choice : null,
			device: { state: this.state, mac: this.mac, name: this.name, detail: { ...this.detail } as Record<string, unknown> },
			log: this.log,
			selftest: this.selftest,
			page: {
				visibility: document.visibilityState,
				standalone: typeof matchMedia === 'function' && matchMedia('(display-mode: standalone)').matches,
			},
		});
	}

	/** Run (or load) the media-element self-test in the background; no user gesture needed. */
	prepare(): Promise<void> {
		this.#selftestRunning ??= (async () => {
			const caps = detect();
			const fmt = mediaFormatFor(caps);
			if (!fmt) return;
			const cached = loadCached(caps.userAgent);
			if (cached && cached.source === fmt.source && cached.mime === fmt.mime) {
				this.selftest = cached;
				return;
			}
			try {
				const pkg = (await import(/* @vite-ignore */ asset('pkg/anywhere_wasm.js'))) as {
					default: () => Promise<unknown>;
					WasmSegmenter: new (rate: number) => never;
				};
				await pkg.default();
				const t = await runSelfTest(fmt.source, fmt.mime, caps.userAgent, (rate) => new pkg.WasmSegmenter(rate));
				saveCached(t);
				this.selftest = t;
				const ok = t.results.filter((r) => r.ok).map((r) => `${r.rate / 1000}k/${r.code.toString(16)}`);
				this.#note(`media self-test: ${ok.length ? `plays ${ok.join(', ')}` : 'no variant played'}`);
			} catch (e) {
				this.#note(`media self-test failed: ${(e as Error).message}`);
			}
		})();
		return this.#selftestRunning;
	}

	/** The automatic choice, corrected by the self-test and by failures this session. */
	#decide(): { choice: ModeChoice; variant: number } {
		const choice = chooseMode(detect(), detectEnvironment(), loadPreference());
		if (choice.mode !== 'media') return { choice, variant: 0 };
		if (this.#mediaBroken) {
			return { choice: { mode: 'worklet', source: null, reason: 'The media element failed earlier in this session; using Web Audio.' }, variant: 0 };
		}
		const v = chooseVariant(this.selftest);
		if (!this.selftest) {
			return { choice: { mode: 'worklet', source: null, reason: 'Media element not tested yet in this browser; using Web Audio for now.' }, variant: 0 };
		}
		if (!v.media) {
			return { choice: { mode: 'worklet', source: null, reason: 'This browser did not play the media-element test streams; using Web Audio.' }, variant: 0 };
		}
		if (!v.highRates) {
			return { choice: { mode: 'worklet', source: null, reason: 'This browser plays the media element only up to 48 kHz; using Web Audio so every track plays.' }, variant: 0 };
		}
		return { choice, variant: v.code };
	}

	/** Report this browser's diagnostics now (e.g. after signing in). */
	report() {
		if (session.current) this.#reporter.urgent();
	}

	/** Log page lifecycle changes, and recover audio when the page comes back. */
	#watchLifecycle() {
		if (this.#lifecycleBound) return;
		this.#lifecycleBound = true;
		const active = () => !!this.#worker;
		document.addEventListener('visibilitychange', () => {
			if (document.visibilityState === 'hidden' && session.current) this.#reporter.urgent();
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

	/** After the system suspended us: restart the keep-alive and the audio output. */
	async #recover() {
		const media = this.#media;
		if (media) {
			if (media.audio.paused && this.detail.output === 'Playing') {
				media.resume();
				this.#note('media element resumed');
			}
			return;
		}
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
		const g = globalThis as Record<string, unknown>;
		const output = typeof AudioWorkletNode !== 'undefined' || !!g.ManagedMediaSource || !!g.MediaSource;
		return output && typeof Worker !== 'undefined' && typeof WebAssembly !== 'undefined';
	}

	/**
	 * Start playing on this device. Call it from a tap: browsers only let sound start inside
	 * one. `auto` (after a reload, no tap): start anyway, and let the next tap unmute.
	 */
	async start(opts: { auto?: boolean } = {}) {
		if (this.#worker || !session.current) return;
		this.#auto = !!opts.auto;
		this.state = 'starting';
		this.wasActive = false;
		this.#watchLifecycle();
		try {
			const { choice, variant } = this.#decide();
			this.choice = choice;
			this.#note(`playback: ${MODE_LABELS[choice.mode]}. ${choice.reason}`);
			this.#configureAudioSession();
			let port: MessagePort | undefined;
			if (choice.mode === 'media' && choice.source) {
				// The element's play() runs first, inside the tap.
				const out = new MediaOutput({
					source: choice.source,
					mime: choice.mime ?? undefined,
					note: (m) => this.#note(m),
					onFail: (why) => void this.#mediaFailed(why),
				});
				out.onDrained = () => {
					this.#reportPosition();
					if (!this.#drainedSent) this.#worker?.postMessage({ cmd: 'media-drained' });
					this.#drainedSent = true;
					this.#drainedAtEnd = out.bufferedEnd;
				};
				this.#media = out;
				await out.start();
				if (out.audio.paused) this.#resumeOnNextTap();
				this.#lastPos = 0;
				this.#timeline = new Timeline();
				this.#posTimer = setInterval(() => this.#reportPosition(), 250);
			} else {
				// Inside the tap: a playing media element helps keep the page alive in the background.
				if (!(await this.#keepAlive.start())) this.#note('background keep-alive refused by the browser');
				port = await this.#createAudio(44100);
				if (this.#ctx?.state !== 'running') this.#resumeOnNextTap();
			}
			const w = new Worker(asset('player-worker.js'), { type: 'module' });
			this.#worker = w;
			w.onmessage = (e) => void this.#onMessage(e.data);
			w.onerror = (e) => {
				this.#note(`worker error: ${e.message}`);
				this.state = 'error';
			};
			w.postMessage({ cmd: 'start', token: session.token, port, mode: choice.mode, variant }, port ? [port] : []);
			this.#history.active = true;
			if (port) w.postMessage({ cmd: 'rate-ready', rate: 44100 });
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

	/** The media element failed: switch this device to Web Audio and carry on. */
	async #mediaFailed(why: string) {
		if (this.#mediaBroken || !this.#media) return;
		this.#mediaBroken = true;
		this.#note(`media element failed (${why}); switching to Web Audio`);
		this.stop();
		await this.start({ auto: true }); // not inside a tap any more
	}

	/** Restart after a reload, if this device was playing before it. */
	async resumeAfterReload() {
		if (!this.wasActive || this.#worker) return;
		this.#note('resuming after the page reloaded');
		await this.start({ auto: true });
	}

	/** The browser held the sound back (no tap): unmute on the next tap or key press. */
	#resumeOnNextTap() {
		if (this.needsTap) return;
		this.needsTap = true;
		this.#note('the browser held the sound back: tap anywhere to resume');
		const resume = () => {
			document.removeEventListener('pointerdown', resume, true);
			document.removeEventListener('keydown', resume, true);
			this.needsTap = false;
			void this.#keepAlive.start();
			this.#media?.resume();
			void this.#ctx?.resume().then(() => this.#note(`audio resumed (${this.#ctx?.state})`));
		};
		document.addEventListener('pointerdown', resume, true);
		document.addEventListener('keydown', resume, true);
	}

	stop(final: DeviceState = 'stopped') {
		this.#history.active = false;
		this.wasActive = false;
		this.needsTap = false;
		this.#keepAlive.stop();
		clearInterval(this.#posTimer);
		this.#media?.stop();
		this.#media = null;
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
		// Without a tap, resume() waits for one: don't let that hold up starting.
		const resumed = ctx.resume();
		if (this.#auto) await Promise.race([resumed, new Promise((r) => setTimeout(r, 500))]);
		else await resumed;
		return channel.port2;
	}

	/** Media mode: tell the core how much the element has played. */
	#reportPosition() {
		const media = this.#media;
		if (!media || !this.#worker) return;
		const t = media.position;
		const now = performance.now();
		if (t > this.#lastPos) {
			const frames = this.#timeline.frames(this.#lastPos, t);
			this.#lastPos = t;
			this.#advancedAt = now;
			this.#drainedSent = false;
			if (frames > 0) this.#worker.postMessage({ cmd: 'played', frames });
		} else {
			// Re-arm once more audio has been appended (e.g. the final partial frame).
			if (this.#drainedSent && media.bufferedEnd > this.#drainedAtEnd + 1e-3) this.#drainedSent = false;
			if (!this.#drainedSent && !media.audio.paused && media.ahead < 0.1 && now - this.#advancedAt > 1000) {
				// Stalled with (almost) nothing buffered: everything appended has played, or the
				// browser won't play a tiny tail without an end of stream. Browsers don't always
				// fire "waiting" again, so this is detected here.
				this.#drainedSent = true;
				this.#drainedAtEnd = media.bufferedEnd;
				this.#advancedAt = now;
				this.#worker.postMessage({ cmd: 'media-drained' });
			}
		}
	}

	async #onMedia(m: Record<string, unknown>) {
		const media = this.#media;
		if (!media) return;
		const span = m.span as { at?: number; rate?: number } | undefined;
		if (span && typeof span.at === 'number' && typeof span.rate === 'number') this.#timeline.mark(span.at, span.rate);
		if (m.init instanceof Uint8Array) media.append(m.init);
		if (m.segment instanceof Uint8Array) media.append(m.segment);
		if (m.pause) media.pause();
		if (m.play) media.resume();
		if (m.flush) {
			this.#reportPosition();
			const at = await media.flush();
			this.#lastPos = at;
			this.#worker?.postMessage({ cmd: 'media-time', at });
		}
	}

	async #onMessage(m: Record<string, unknown>) {
		if (typeof m.log === 'string') this.#note(m.log);
		if (m.media && typeof m.media === 'object') await this.#onMedia(m.media as Record<string, unknown>);
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
		if (typeof m.rate === 'number' && this.#worker && !this.#media) {
			const port = await this.#createAudio(m.rate);
			if (this.#ctx?.state !== 'running') this.#resumeOnNextTap();
			this.#worker?.postMessage({ cmd: 'start-sink', port }, [port]);
			this.#worker?.postMessage({ cmd: 'rate-ready', rate: m.rate });
		}
		// The worker reports status every 20 ms; the UI only needs a few updates a second.
		if (m.status && typeof m.status === 'object' && performance.now() - this.#detailAt > 500) {
			this.#detailAt = performance.now();
			this.detail = m.status as DeviceStatus;
		}
	}
}

export const device = new DevicePlayer();
