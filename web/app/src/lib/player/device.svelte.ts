// "This device": the browser as an LMS player. The WASM core runs in a worker, which owns
// the gateway socket and stream downloads. Audio plays through an AudioWorklet, or through an
// <audio> element (Media Source) where only that keeps playing in the background (iOS).
import { version } from '$app/env';
import { api, session } from '#lib/state/app.svelte';
import { KeepAlive, PersistentLog, describeLifecycle } from './background';
import { canPlayHere, detect } from './capabilities';
import { MediaOutput, Timeline } from './media-output';
import { PlayheadSmoother, outputClock } from './output-clock';
import { MEDIA_TYPE, MODE_LABELS, chooseMode, detectEnvironment, loadPreference, type ModeChoice } from './mode';
import { LogLimiter, Reporter, buildReport, describePageCommand, diagnosticsSender } from './diagnostics';
import { fadeGain } from '#lib/lms/sleep';
import { nowPlaying as briefStatus, volumeControl } from '#lib/lms/commands';
import { mayResumeMedia } from './local-plan';
import { needsGesture, rememberAutoplay } from './autoplay';
import {
	connection,
	effectiveQuality,
	loadPreference as loadQualityPreference,
	savePreference as saveQualityPreference,
	type QualityPreference,
	type StreamQuality,
} from './stream-quality';
import { chooseVariant, loadCached, mediaFormatFor, mimeFor, runSelfTest, saveCached, type SelfTest } from './selftest';
import {
	ElementVolume,
	VOLUME_LABELS,
	belowFull,
	cleanGains,
	detectMediaVolume,
	elementVolume,
	fixedLevelChange,
	loadFixedLevel,
	loadVolumeSetting,
	saveFixedLevel,
	outputGains,
	planVolume,
	saveVolumeSetting,
	type VolumeAbility,
	type VolumePlan,
	type VolumeSetting,
} from './volume';

/** The Web Lock a tab holds while it runs this device's player. */
const DEVICE_LOCK = 'anywhere-device-player';

/** While LMS's volume is held at 100% here, ask again at most this often. */
const HOLD_FULL_EVERY_MS = 3000;
/** While a controller has changed this player's Volume Control against the setting, set it back at most this often. */
export const FIXED_LEVEL_EVERY_MS = 30_000;
/** Web Audio: tell the worker when the output is heard this often (for sync). */
const CLOCK_EVERY_MS = 250;
/** Media element: read the play head this often (the background may slow it to 1 s). */
const POSITION_EVERY_MS = 100;
/** A smoothed play head is never put further ahead of the element's own than this (s). */
const SMOOTH_MAX_LEAD_S = 0.03;

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
	/** Paused here while the server is out of reach (the output is held; see local.js). */
	held?: boolean;
	/** The stream being fetched has been received to its end. */
	received?: boolean;
	/** The core's sequence number of the track last handed to the output. */
	track?: number | null;
	/** What the engine is decoding (`codec`, `lossless`, `bits`); checked by `streamOf` before use. */
	source?: unknown;
}

/** A player file, with the build version so a stale cached copy is never used. */
const asset = (p: string) => {
	const u = new URL(`player/${p}`, document.baseURI);
	if (/\.(js|wasm)$/.test(p)) u.searchParams.set('v', version);
	return u.toString();
};

class DevicePlayer {
	state = $state<DeviceState>('off');
	mac = $state('');
	name = $state('');
	detail = $state<DeviceStatus>({});
	/** Run when this device's output goes from not playing to playing (see `onPlaying`). */
	#onPlaying = new Set<() => void>();
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
	/** Media mode, started without a tap: the browser's answer to its play() is remembered. */
	#autoProbe = false;
	/** Stops listening for the tap that unmutes (see #resumeOnNextTap). */
	#tapCleanup: (() => void) | null = null;
	/** Called when the player is connected (the app selects it). */
	onConnected: ((mac: string) => void) | null = null;
	/** The last connection was a fresh session (LMS saw a new player), not a resumed one. */
	freshSession = $state(false);
	/** A pause made here is holding the output (until the worker lets it go). */
	#holding = false;
	/** "Volume on this device" in Settings. */
	volumeSetting = $state<VolumeSetting>(loadVolumeSetting());
	/** "Fix this player's output level at 100%" in Settings (on by default; see volume.ts). */
	fixedLevel = $state(loadFixedLevel());
	/** Called after this player's Volume Control was changed in LMS (the app refreshes its status). */
	onLevelChanged: (() => void) | null = null;
	/** When this player's Volume Control was last set (performance.now()). */
	#fixedAt = -Infinity;
	/** A read-and-set of the Volume Control is under way. */
	#fixedBusy = false;
	/** "Streaming quality in this browser" in Settings, and the level it means now. */
	qualityPreference = $state<QualityPreference>(loadQualityPreference());
	quality = $state<StreamQuality>('original');
	#qualityWatch = false;
	/** How volume works while this device plays (null when it isn't playing). */
	volumePlan = $state<VolumePlan | null>(null);
	/** LMS's volume for this player, as linear [left, right] gains. */
	#lmsGains: [number, number] = [1, 1];
	/** LMS's volume as a linear gain (the louder side), for the signal path. */
	gain = $state(1);
	/** The sleep timer's fade-out: a factor on the output, 1 when none runs. */
	#fade = 1;
	#fadeTimer: ReturnType<typeof setInterval> | undefined;
	#fadeKey = '';
	/** The output device's own sample rate (from a default AudioContext), once read. */
	#deviceRate: number | null | undefined;
	#mediaVolume: boolean | null = null;
	#elementVolume: ElementVolume | null = null;
	#node: AudioWorkletNode | null = null;
	#heldFullAt = -Infinity;
	/** Media mode: the next timeline position from the worker follows a flush. */
	#seekAfterFlush = false;
	#restartTimer: ReturnType<typeof setInterval> | undefined;
	/** Web Audio: posts the output clock to the worker. */
	#clockTimer: ReturnType<typeof setInterval> | undefined;
	/** Media element: a synced start waiting for its time. */
	#startTimer: ReturnType<typeof setTimeout> | undefined;
	#smoother = new PlayheadSmoother();

	#worker: Worker | null = null;
	/** Who wants visualiser frames (Wave 2 §4.1); the worker analyses only while someone does. */
	#visualWatchers = new Set<(frame: Float32Array) => void>();
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
		diagnosticsSender((fn) => session.authed(fn)),
		() => this.#diagnostics(),
	);
	/** Bumped by every start and stop: work a stopped start was still doing is dropped. */
	#gen = 0;
	/** The rate of the AudioContext in use or being built (0: none). */
	#audioRate = 0;
	/** AudioContexts are built one at a time, in order. */
	#audioQueue: Promise<void> = Promise.resolve();

	/** Lines of a kind logged moments ago are left out (see LogLimiter). */
	#limiter = new LogLimiter();

	#note(msg: string, kind?: string) {
		const line = this.#limiter.pass(msg, kind, performance.now());
		if (line === null) return;
		this.log = this.#history.add(line);
		if (session.current) this.#reporter.schedule();
	}

	/** A line for this device's connection log, from elsewhere in the app (e.g. a command it sent LMS by itself). */
	note(msg: string) {
		this.#note(msg);
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
					default: (o: { module_or_path: string }) => Promise<unknown>;
					WasmSegmenter: new (rate: number) => never;
				};
				await pkg.default({ module_or_path: asset('pkg/anywhere_wasm_bg.wasm') });
				const t = await runSelfTest(fmt.source, fmt.mime, caps.userAgent, (rate) => new pkg.WasmSegmenter(rate));
				// Only a browser's real verdict is worth keeping, not our own setup failing.
				if (!t.results.some((r) => r.setup)) saveCached(t);
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
		// The variant decides the codec: Apple Lossless is appended under its own type.
		return { choice: { ...choice, mime: mimeFor(v.code, choice.mime ?? MEDIA_TYPE) }, variant: v.code };
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
			if (media.audio.paused && this.detail.output === 'Playing' && mayResumeMedia(this.detail.output, this.held)) {
				// Noted once it really plays; a refusal asks for a tap (see MediaOutput.onBlocked).
				void media.resume().then((ok) => ok && this.#note('media element resumed'));
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

	/** Is the output held by a pause made here? */
	get held(): boolean {
		return this.#holding || this.detail.held === true;
	}

	/**
	 * Pause (`hold`) or resume this device's own output while the server is out of reach,
	 * keeping the position; or let a hold go once LMS has taken over (see local.js).
	 */
	local(m: { hold: true } | { release: 'now' | 'flush' | 'paused' }) {
		if (!this.#worker) return;
		this.#holding = 'hold' in m;
		this.#worker.postMessage({ cmd: 'local', ...m });
		if (this.#holding) this.#media?.pause();
	}

	/** What this browser can do about volume, for the mode in use (or planned). */
	volumeAbility(): VolumeAbility {
		this.#mediaVolume ??= typeof Audio === 'function' ? detectMediaVolume() : false;
		const mode = this.#worker && this.choice ? this.choice.mode : chooseMode(detect(), detectEnvironment(), loadPreference()).mode;
		return { mode, mediaVolume: this.#mediaVolume };
	}

	/** Change "Streaming quality in this browser"; applies from the next track. */
	setQualityPreference(p: QualityPreference) {
		this.qualityPreference = p;
		saveQualityPreference(p);
		this.#updateQuality();
	}

	/** The level for the preference and the network now; tells the player when it changes. */
	#updateQuality() {
		const c = connection();
		if (c && !this.#qualityWatch) {
			// Follow the network (Wi-Fi to mobile data and back) while the page is open.
			this.#qualityWatch = true;
			c.addEventListener?.('change', () => this.#updateQuality());
		}
		const q = effectiveQuality(this.qualityPreference, c);
		if (q === this.quality) return;
		this.quality = q;
		this.#worker?.postMessage({ cmd: 'quality', quality: q });
	}

	/** Change "Volume on this device"; applies at once if this device is playing. */
	setVolumeSetting(s: VolumeSetting) {
		this.volumeSetting = s;
		saveVolumeSetting(s);
		if (!this.#worker || !this.choice) return;
		this.#planVolume(this.choice.mode);
		this.#applyVolume();
	}

	/**
	 * Change "Fix this player's output level at 100%": kept per browser, and set on this
	 * device's player in LMS at once if it is connected.
	 */
	setFixedLevel(on: boolean) {
		this.fixedLevel = on;
		saveFixedLevel(on);
		if (this.state === 'connected' && this.mac) void this.#syncFixedLevel('the setting changed');
	}

	/**
	 * Bring LMS's Volume Control for this player in step with the setting: read it, and set it
	 * if they disagree. One attempt per call: a failure is noted, never retried here.
	 */
	async #syncFixedLevel(why: string) {
		const mac = this.mac;
		const gen = this.#gen;
		if (!mac || this.#fixedBusy) return;
		this.#fixedBusy = true;
		try {
			let reported: unknown;
			try {
				reported = (await api.rpc<{ digital_volume_control?: unknown }>(mac, briefStatus()))?.digital_volume_control;
			} catch (e) {
				this.#note(`could not read this player’s Volume Control: ${(e as Error).message}`);
				return;
			}
			if (gen !== this.#gen || mac !== this.mac) return;
			const v = fixedLevelChange(this.fixedLevel, reported);
			if (v) await this.#setVolumeControl(mac, v, why);
		} finally {
			this.#fixedBusy = false;
		}
	}

	async #setVolumeControl(mac: string, v: '0' | '1', why: string) {
		this.#fixedAt = performance.now();
		const c = volumeControl(v);
		this.#note(describePageCommand(c, why), 'page:volume-control');
		try {
			await api.rpc(mac, c);
			this.onLevelChanged?.();
		} catch (e) {
			this.#note(`could not set this player’s Volume Control: ${(e as Error).message}`);
		}
	}

	/**
	 * The app's status for the player it controls. When that is this device's and a
	 * controller has changed its Volume Control against the setting, set it back (at most
	 * every {@link FIXED_LEVEL_EVERY_MS}).
	 */
	observeStatus(playerId: string | null | undefined, status: { digital_volume_control?: unknown } | null | undefined) {
		if (!playerId || playerId !== this.mac || this.state !== 'connected' || this.#fixedBusy) return;
		const v = fixedLevelChange(this.fixedLevel, status?.digital_volume_control);
		if (!v || performance.now() - this.#fixedAt < FIXED_LEVEL_EVERY_MS) return;
		void this.#setVolumeControl(this.mac, v, `a controller changed it; the setting ${v === '0' ? 'fixes the output level' : 'lets LMS set the volume'}`);
	}

	/** True when `playerId` is this device and its own volume buttons set the level. */
	buttonsOnly(playerId: string | null | undefined): boolean {
		return !!playerId && playerId === this.mac && !!this.volumePlan && !this.volumePlan.follow;
	}

	#planVolume(mode: VolumeAbility['mode']) {
		this.#mediaVolume ??= typeof Audio === 'function' ? detectMediaVolume() : false;
		const plan = planVolume({ mode, mediaVolume: this.#mediaVolume }, this.volumeSetting);
		this.volumePlan = plan;
		this.#note(`volume: ${VOLUME_LABELS[this.volumeSetting]} (${plan.follow ? 'follows LMS' : 'device buttons; LMS held at 100%'}). ${plan.reason}`);
	}

	/** Apply LMS's volume (or full scale) at the output, after anything queued. */
	#applyVolume() {
		const plan = this.volumePlan;
		const [l, r] = outputGains(plan, this.#lmsGains);
		const gains: [number, number] = [l * this.#fade, r * this.#fade];
		if (this.#media) this.#elementVolume?.set(elementVolume(gains));
		else this.#node?.port.postMessage({ gain: gains });
		if (plan && !plan.follow) this.#holdLmsAtFull();
	}

	/**
	 * Device buttons only: LMS's volume is held at 100%, so every controller shows what is
	 * heard. A controller that moves it is answered by setting it back (rate-limited; a
	 * mute is left alone).
	 */
	#holdLmsAtFull() {
		const g = this.#lmsGains;
		if (!belowFull(g) || (g[0] === 0 && g[1] === 0)) return;
		if (!this.mac || this.state !== 'connected') return;
		const now = performance.now();
		if (now - this.#heldFullAt < HOLD_FULL_EVERY_MS) return;
		this.#heldFullAt = now;
		this.#note(describePageCommand(['mixer', 'volume', 100], 'device buttons only: LMS is held at full'), 'page:volume');
		api.rpc(this.mac, ['mixer', 'volume', 100]).catch((e: Error) => this.#note(`could not set LMS volume to 100%: ${e.message}`));
	}

	/**
	 * Fade this device's output out before its sleep timer ends (`endsAt`, ms on Date.now()'s
	 * clock), over `fadeS` seconds; null cancels. The fade lifts by itself shortly after the
	 * end, once LMS has stopped the player. LMS also lowers the volume over the last minute
	 * (its own sleep fade), which this device follows when its volume follows LMS.
	 */
	setSleepFade(endsAt: number | null, fadeS: number) {
		const key = endsAt === null || fadeS <= 0 ? '' : `${Math.round(endsAt / 1000)}:${fadeS}`;
		if (key === this.#fadeKey) return;
		this.#fadeKey = key;
		clearInterval(this.#fadeTimer);
		this.#fadeTimer = undefined;
		const set = (f: number) => {
			if (Math.abs(f - this.#fade) < 0.002 && f !== 0 && f !== 1) return;
			if (f === this.#fade) return;
			this.#fade = f;
			this.#applyVolume();
		};
		if (!key || endsAt === null) {
			set(1);
			return;
		}
		this.#note(`sleep timer: fading out over ${fadeS} s before it ends`);
		const step = () => {
			const left = (endsAt - Date.now()) / 1000;
			if (left < -10) {
				// Well past the end: LMS has stopped the player. Back to full for next time.
				clearInterval(this.#fadeTimer);
				this.#fadeTimer = undefined;
				this.#fadeKey = '';
				set(1);
				return;
			}
			set(fadeGain(left, fadeS));
		};
		step();
		this.#fadeTimer = setInterval(step, 250);
	}

	/** The output device's own sample rate, or null if the browser won't say. */
	deviceRate(): number | null {
		if (this.#deviceRate !== undefined) return this.#deviceRate;
		this.#deviceRate = null;
		try {
			if (typeof AudioContext === 'function') {
				const ctx = new AudioContext();
				this.#deviceRate = Number.isFinite(ctx.sampleRate) && ctx.sampleRate > 0 ? ctx.sampleRate : null;
				ctx.close().catch(() => {});
			}
		} catch {
			/* not available */
		}
		return this.#deviceRate;
	}

	get supported(): boolean {
		return canPlayHere();
	}

	/** Releases this tab's hold on the device player (see #holdTab). */
	#releaseTab: (() => void) | null = null;

	/**
	 * Every tab of this browser is the same device (the same LMS player), so only one may run
	 * it. A tab holds a Web Lock while its player runs: a tab that starts by itself only does
	 * so if no other tab holds it; a tap takes it over (the gateway then closes the other
	 * tab's player). Browsers without Web Locks just start.
	 */
	#holdTab(takeOver: boolean): Promise<boolean> {
		if (this.#releaseTab) return Promise.resolve(true);
		const locks = (globalThis.navigator as Navigator | undefined)?.locks;
		if (!locks?.request) return Promise.resolve(true);
		return new Promise<boolean>((resolve) => {
			locks
				.request(DEVICE_LOCK, takeOver ? { steal: true } : { ifAvailable: true }, (lock) => {
					if (!lock) {
						resolve(false);
						return undefined;
					}
					resolve(true);
					return new Promise<void>((release) => (this.#releaseTab = release));
				})
				.catch(() => {
					// Taken over by another tab, or locks unavailable here.
					this.#releaseTab = null;
					resolve(takeOver);
				});
		});
	}

	/**
	 * Start by itself, without a tap (after signing in, or a reload): the player connects and
	 * is ready, and the first tap anywhere turns the sound on. Not if another tab of this
	 * browser already runs it ('busy').
	 */
	async autoStart(): Promise<'started' | 'busy' | 'unsupported' | 'failed'> {
		if (!this.supported) return 'unsupported';
		if (this.#worker || this.state === 'starting') return 'started';
		if (!(await this.#holdTab(false))) return 'busy';
		if (this.wasActive) this.#note('resuming after the page reloaded');
		await this.start({ auto: true });
		return this.state === 'error' ? 'failed' : 'started';
	}

	/**
	 * Start playing on this device. Call it from a tap: browsers only let sound start inside
	 * one. `auto` (after a reload, no tap): start anyway, and let the next tap unmute.
	 */
	async start(opts: { auto?: boolean } = {}) {
		if (this.#worker || this.state === 'starting' || !session.current) return;
		if (!opts.auto) void this.#holdTab(true);
		// Stop may be pressed while this is still starting: after every wait, check that this
		// start is still the current one, and if not, drop what it built and go no further.
		const gen = ++this.#gen;
		const live = () => gen === this.#gen;
		this.#auto = !!opts.auto;
		this.state = 'starting';
		this.wasActive = false;
		this.#watchLifecycle();
		try {
			const { choice, variant } = this.#decide();
			this.choice = choice;
			this.#note(`playback: ${MODE_LABELS[choice.mode]}. ${choice.reason}`);
			this.#lmsGains = [1, 1];
			this.#heldFullAt = -Infinity;
			this.#fixedAt = -Infinity;
			this.#planVolume(choice.mode);
			this.#configureAudioSession();
			let port: MessagePort | undefined;
			if (choice.mode === 'media' && choice.source) {
				// The element's play() runs first, inside the tap. Without one, on a browser known
				// to refuse it, the element waits for the tap and says so at once.
				const defer = !!opts.auto && needsGesture();
				this.#autoProbe = !!opts.auto && !defer;
				const out = new MediaOutput({
					source: choice.source,
					mime: choice.mime ?? undefined,
					note: (m, kind) => this.#note(m, kind),
					onFail: (why) => void this.#mediaFailed(why),
					deferPlay: defer,
				});
				out.onBlocked = () => {
					if (this.#media !== out) return;
					if (this.#autoProbe) rememberAutoplay(true);
					this.#autoProbe = false;
					this.#resumeOnNextTap();
				};
				out.onPlaying = () => {
					if (this.#media !== out) return;
					if (this.#autoProbe) rememberAutoplay(false);
					this.#autoProbe = false;
					this.#soundOn();
				};
				out.onExternalPause = () => {
					// The system paused us (a call, another app's audio): pause the player in LMS
					// too, so every controller shows it paused and Play resumes it, rather than
					// LMS carrying on as if it were playing while nothing is heard.
					if (!this.mac || this.detail.output !== 'Playing') return;
					this.#note(describePageCommand(['pause', 1], 'the system paused the media element'));
					api.rpc(this.mac, ['pause', 1]).catch((e: Error) => this.#note(`could not pause the player: ${e.message}`));
				};
				out.onDrained = () => {
					this.#reportPosition();
					if (!this.#drainedSent) this.#worker?.postMessage({ cmd: 'media-drained' });
					this.#drainedSent = true;
					this.#drainedAtEnd = out.bufferedEnd;
				};
				this.#media = out;
				this.#elementVolume = new ElementVolume(out.audio);
				this.#applyVolume();
				if (defer) {
					this.#note('this browser plays only after a tap: waiting for one');
					this.#resumeOnNextTap();
				}
				await out.start();
				if (!live()) {
					out.stop(); // stop() has stopped it already; this is harmless
					return;
				}
				if (out.audio.paused) this.#resumeOnNextTap();
				this.#lastPos = 0;
				this.#timeline = new Timeline();
				this.#posTimer = setInterval(() => this.#reportPosition(), POSITION_EVERY_MS);
			} else {
				// Inside the tap: a playing media element helps keep the page alive in the background.
				const kept = await this.#keepAlive.start();
				if (!live()) return;
				if (!kept) this.#note('background keep-alive refused by the browser');
				this.#audioRate = 44100;
				const p = await this.#createAudio(44100, gen);
				if (!p || !live()) return; // #createAudio closes its context when stopped
				port = p;
				if (this.#ctx?.state !== 'running') this.#resumeOnNextTap();
			}
			const w = new Worker(asset('player-worker.js'), { type: 'module' });
			this.#worker = w;
			w.onmessage = (e) => {
				if (this.#worker === w) void this.#onMessage(e.data);
			};
			w.onerror = (e) => {
				if (this.#worker !== w) return;
				// The worker is gone (e.g. its script failed to load): release everything, so
				// "Play on this device" can start afresh. The log keeps the reason.
				this.#note(`worker error: ${e.message}`);
				this.stop('error');
			};
			this.#updateQuality();
			w.postMessage({ cmd: 'start', token: session.token, port, mode: choice.mode, variant, quality: this.quality }, port ? [port] : []);
			if (this.#visualWatchers.size) w.postMessage({ cmd: 'visual', on: true });
			this.#history.active = true;
			if (port) {
				w.postMessage({ cmd: 'rate-ready', rate: 44100 });
				this.#postClock();
				this.#clockTimer = setInterval(() => this.#postClock(), CLOCK_EVERY_MS);
			}
			// Pass renewed access tokens to the worker; stop if signed out.
			this.#unsubscribe = session.subscribe((s) => {
				if (!s) this.stop();
				else this.#worker?.postMessage({ cmd: 'token', token: s.access_token });
			});
		} catch (e) {
			if (!live()) return; // stopped meanwhile: not an error
			this.#note(`could not start: ${(e as Error).message}`);
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
		await this.autoStart();
	}

	/**
	 * The browser held the sound back (no tap): unmute on the next tap or key press. In media
	 * mode the hold lasts until the element really plays (#soundOn): a tap whose play() is
	 * aborted (a stream restart at the same moment) leaves the element to try again by itself
	 * once the new audio is in, and a refused one keeps waiting for the next tap.
	 */
	#resumeOnNextTap() {
		if (this.needsTap) return;
		this.needsTap = true;
		this.#note('the browser held the sound back: tap anywhere to resume');
		// Browsers only count some events as a user gesture: for touch, the finger lifting
		// (pointerup/touchend/click), not touching down. Listen to all of them, and keep
		// listening until the sound really is on.
		const events = ['pointerup', 'touchend', 'click', 'keydown'] as const;
		const resume = async () => {
			if (!this.needsTap) return this.#tapCleanup?.();
			void this.#keepAlive.start();
			const media = this.#media;
			if (media) {
				// Held by a pause made here: the element is meant to be paused.
				if (this.held) return this.#soundOn();
				// Not while LMS has the player paused: that would play the old queued audio. The
				// hold ends when it really plays (onPlaying), not when it is asked to.
				if (mayResumeMedia(String(this.detail.output ?? ''), this.held)) await media.resume();
				return;
			}
			await this.#ctx?.resume().catch(() => {});
			if (this.#ctx && this.#ctx.state !== 'running') return;
			this.#soundOn();
		};
		const onTap = () => void resume();
		for (const t of events) document.addEventListener(t, onTap, true);
		this.#tapCleanup = () => {
			for (const t of events) document.removeEventListener(t, onTap, true);
			this.#tapCleanup = null;
		};
	}

	/** The sound is really on: end the hold, if there is one. */
	#soundOn() {
		if (!this.needsTap) return;
		this.#tapCleanup?.();
		this.needsTap = false;
		this.#note(`audio resumed (${this.#ctx?.state ?? 'media'})`);
	}

	stop(final: DeviceState = 'stopped') {
		this.#gen++;
		this.#audioRate = 0;
		this.#releaseTab?.();
		this.#releaseTab = null;
		this.#history.active = false;
		this.wasActive = false;
		this.needsTap = false;
		this.#tapCleanup?.();
		this.#autoProbe = false;
		this.#keepAlive.stop();
		clearInterval(this.#posTimer);
		clearInterval(this.#restartTimer);
		clearInterval(this.#clockTimer);
		clearTimeout(this.#startTimer);
		this.#smoother.reset();
		this.#seekAfterFlush = false;
		this.#elementVolume?.cancel();
		this.#elementVolume = null;
		this.#node = null;
		this.volumePlan = null;
		this.#media?.stop();
		this.#media = null;
		this.#unsubscribe?.();
		this.#unsubscribe = null;
		this.#worker?.terminate();
		this.#worker = null;
		this.#holding = false;
		this.#ctx?.close().catch(() => {});
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

	/**
	 * A new AudioContext at `rate`, replacing (and first closing) the current one. Returns
	 * null, with the new context closed again, if the device was stopped meanwhile.
	 * Callers run one at a time (see #onRate).
	 */
	async #createAudio(rate: number, gen: number): Promise<MessagePort | null> {
		const live = () => gen === this.#gen;
		const old = this.#ctx;
		this.#ctx = null;
		if (old) await old.close().catch(() => {});
		if (!live()) return null;
		// The core outputs at the track's own rate; the browser resamples to the hardware.
		const ctx = new AudioContext({ sampleRate: rate, latencyHint: 'playback' });
		this.#ctx = ctx;
		const abandon = () => {
			if (this.#ctx === ctx) this.#ctx = null;
			ctx.close().catch(() => {});
			return null;
		};
		try {
			// Withheld on plain http:// (an insecure context); detection should have chosen the
			// media element, so say clearly what is missing rather than fail on undefined.
			if (!ctx.audioWorklet) throw new Error('Web Audio worklets are not available on this page (they need HTTPS)');
			await ctx.audioWorklet.addModule(asset('worklet.js'));
		} catch (e) {
			abandon();
			throw e;
		}
		if (!live()) return abandon();
		const node = new AudioWorkletNode(ctx, 'anywhere-sink', { outputChannelCount: [2] });
		node.connect(ctx.destination);
		this.#node = node;
		node.port.postMessage({ gain: outputGains(this.volumePlan, this.#lmsGains), instant: true });
		const channel = new MessageChannel();
		node.port.postMessage({ port: channel.port1 }, [channel.port1]);
		// Without a tap, resume() waits for one: don't let that hold up starting.
		const resumed = ctx.resume();
		if (this.#auto) await Promise.race([resumed, new Promise((r) => setTimeout(r, 500))]);
		else await resumed;
		if (!live()) return abandon();
		return channel.port2;
	}

	/**
	 * The core's output rate changed: build an AudioContext at that rate. Requests are
	 * handled one at a time; a repeat of the rate already in use (or being built) is
	 * ignored, and a request overtaken by a newer one is dropped before it builds anything.
	 */
	#onRate(rate: number): Promise<void> {
		if (rate === this.#audioRate || !this.#worker || this.#media) return this.#audioQueue;
		this.#audioRate = rate;
		const gen = this.#gen;
		const job = this.#audioQueue.then(async () => {
			if (gen !== this.#gen || rate !== this.#audioRate) return; // stopped, or overtaken
			const port = await this.#createAudio(rate, gen);
			if (!port || gen !== this.#gen) return;
			if (rate !== this.#audioRate) {
				port.close(); // overtaken while building: the next request replaces it
				return;
			}
			if (this.#ctx?.state !== 'running') this.#resumeOnNextTap();
			this.#worker?.postMessage({ cmd: 'start-sink', port }, [port]);
			this.#worker?.postMessage({ cmd: 'rate-ready', rate });
			this.#postClock();
		});
		this.#audioQueue = job.catch((e) => {
			if (gen !== this.#gen) return;
			this.#audioRate = 0; // let the worker's next request try again
			this.#note(`audio output failed at ${rate} Hz: ${(e as Error).message}`);
		});
		return this.#audioQueue;
	}

	/**
	 * Media mode, after a flush: report the position often until the new audio plays, so
	 * LMS (and the seek timing in the log) hear of it promptly, not at the next 250 ms poll.
	 */
	#watchRestart() {
		clearInterval(this.#restartTimer);
		const until = performance.now() + 1500;
		this.#restartTimer = setInterval(() => {
			this.#reportPosition();
			if (!this.#media || performance.now() > until) clearInterval(this.#restartTimer);
		}, 20);
	}

	/**
	 * Web Audio: tell the worker which context time is heard when, so it reports what is
	 * audible rather than what has been rendered (see output-clock.ts).
	 */
	#postClock() {
		const ctx = this.#ctx;
		const w = this.#worker;
		if (!ctx || !w || ctx.state !== 'running') return;
		const c = outputClock(ctx, performance.timeOrigin, performance.now());
		if (c) w.postMessage({ cmd: 'clock', ...c });
	}

	/**
	 * Media mode: tell the core how much the element has played, and when that was read. The
	 * play head is smoothed while playing (see output-clock.ts); a report of no new frames
	 * still says where it is now.
	 */
	#reportPosition() {
		const media = this.#media;
		if (!media || !this.#worker) return;
		const raw = media.position;
		const now = performance.now();
		const at = performance.timeOrigin + now;
		let t = raw;
		const playing = !media.audio.paused;
		if (playing) t = Math.min(this.#smoother.update(raw, at / 1000), raw + SMOOTH_MAX_LEAD_S);
		else {
			this.#smoother.reset();
			// Paused: what was heard ends where it paused (nothing, if it never started). A play
			// head that moved since (a seek, the browser snapping it onto buffered audio) was not
			// heard, and the core must never count audio that did not play.
			t = Math.min(raw, media.pausedAt ?? this.#lastPos);
		}
		if (t > this.#lastPos) {
			const frames = this.#timeline.frames(this.#lastPos, t);
			this.#lastPos = t;
			this.#advancedAt = now;
			this.#drainedSent = false;
			this.#worker.postMessage({ cmd: 'played', frames, at });
		} else {
			this.#worker.postMessage({ cmd: 'played', frames: 0, at });
			// Re-arm once more audio has been appended (e.g. the final partial frame).
			if (this.#drainedSent && media.bufferedEnd > this.#drainedAtEnd + 1e-3) this.#drainedSent = false;
			if (!this.#drainedSent && media.atEnd && now - this.#advancedAt > 1000) {
				// Stalled at the end of everything appended: it has all played, or the browser
				// won't play a tiny tail without an end of stream. Browsers don't always fire
				// "waiting" again, so this is detected here. (A stall anywhere else is not the
				// end: counting it as one would report audio still to come as played.)
				this.#drainedSent = true;
				this.#drainedAtEnd = media.bufferedEnd;
				this.#advancedAt = now;
				this.#worker.postMessage({ cmd: 'media-drained' });
			} else if (!media.audio.paused && now - this.#advancedAt > 1000 && (media.unstick() || media.kick(now))) {
				// A gap just ahead, or stuck inside buffered audio: moved on, or kicked.
				this.#advancedAt = now;
			}
		}
		if (!playing && raw > this.#lastPos) this.#lastPos = raw; // moved, unheard: skip it
	}

	async #onMedia(m: Record<string, unknown>) {
		const media = this.#media;
		if (!media) return;
		const span = m.span as { at?: number; rate?: number; keepBehind?: number } | undefined;
		if (span && typeof span.keepBehind === 'number') media.setKeepBehind(span.keepBehind);
		if (span && typeof span.at === 'number' && typeof span.rate === 'number') {
			this.#timeline.mark(span.at, span.rate);
			if (this.#seekAfterFlush) {
				// The new audio starts exactly here: move the play head to it (see MediaOutput.seek).
				this.#seekAfterFlush = false;
				media.seek(span.at);
				this.#lastPos = span.at;
				this.#watchRestart();
			}
		}
		// LMS dropped the tracks after the one playing: cut them out (before any new segments).
		if (typeof m.trimFrom === 'number') media.trimFrom(m.trimFrom);
		if (m.init instanceof Uint8Array) media.append(m.init);
		if (m.segment instanceof Uint8Array) media.append(m.segment);
		if (m.pause) media.pause();
		// Refused for want of a gesture: only the tap can start it (see #resumeOnNextTap).
		if (m.play) {
			clearTimeout(this.#startTimer);
			if (!media.blocked) void media.resume();
		}
		// LMS's sync skips and pauses never come here: the worker makes them in the audio
		// before muxing (static/player/media-sync.js). Seeking or pausing the element for them
		// stalled it on iPadOS, so LMS asked again at every check.
		if (typeof m.startAt === 'number' && Number.isFinite(m.startAt)) {
			// A synced start: play at that moment, not at the worker's next tick after it.
			const wait = m.startAt - (performance.timeOrigin + performance.now());
			clearTimeout(this.#startTimer);
			if (wait > 0 && wait < 30_000) this.#startTimer = setTimeout(() => this.#media === media && mayResumeMedia('Playing', this.held) && !media.blocked && media.resume(), wait);
		}
		if (m.flush) {
			clearTimeout(this.#startTimer);
			this.#smoother.reset();
			this.#reportPosition();
			const at = await media.flush();
			this.#lastPos = at;
			this.#seekAfterFlush = true;
			this.#worker?.postMessage({ cmd: 'media-time', at });
		}
	}

	/**
	 * Visualiser frames (see static/player/visual.js) while `cb` watches: the worker analyses
	 * the audio only while at least one watcher is registered. Returns the unwatch function.
	 */
	watchVisual(cb: (frame: Float32Array) => void): () => void {
		this.#visualWatchers.add(cb);
		if (this.#visualWatchers.size === 1) this.#worker?.postMessage({ cmd: 'visual', on: true });
		return () => {
			if (!this.#visualWatchers.delete(cb)) return;
			if (!this.#visualWatchers.size) this.#worker?.postMessage({ cmd: 'visual', on: false });
		};
	}

	/** How many watch the visualiser (for tests and diagnostics). */
	get visualWatchers(): number {
		return this.#visualWatchers.size;
	}

	async #onMessage(m: Record<string, unknown>) {
		if (m.visual instanceof Float32Array) {
			for (const cb of this.#visualWatchers) cb(m.visual);
			return;
		}
		if (typeof m.log === 'string') this.#note(m.log, typeof m.kind === 'string' ? m.kind : undefined);
		if (Array.isArray(m.volume)) {
			this.#lmsGains = cleanGains(m.volume);
			this.gain = Math.max(...this.#lmsGains);
			this.#applyVolume();
		}
		if (m.media && typeof m.media === 'object') await this.#onMedia(m.media as Record<string, unknown>);
		if (m.connection === 'connected') {
			this.state = 'connected';
			this.freshSession = m.fresh === true;
			const p = m.player as { mac?: string; name?: string } | undefined;
			if (p?.mac && p.mac !== this.mac) {
				this.mac = p.mac;
				this.name = p.name ?? '';
				this.onConnected?.(p.mac);
			}
			this.#applyVolume();
			void this.#syncFixedLevel('this player connected');
		} else if (m.connection === 'reconnecting' || m.connection === 'restarting' || m.connection === 'reauthenticating') {
			this.state = 'reconnecting';
		} else if (m.connection === 'closed') {
			// The gateway ended this player for good (e.g. it was replaced elsewhere): release
			// the worker and audio, so "Play on this device" can start again.
			this.#note('the gateway closed this player');
			this.stop();
			return;
		}
		if (m.authFailed) {
			// The access token was refused (expired): renew it and let the worker carry on. If
			// the server can't be reached, the session retries, and the worker reconnects on
			// the token that brings (it waits for any new token).
			const worker = this.#worker;
			if ((await session.renew('player')) && worker && this.#worker === worker) {
				worker.postMessage({ cmd: 'token', token: session.token, reconnect: true });
			}
		}
		if (typeof m.rate === 'number' && Number.isFinite(m.rate) && m.rate > 0) await this.#onRate(m.rate);
		// The worker reports status every 20 ms; the UI only needs a few updates a second.
		if (m.status && typeof m.status === 'object' && performance.now() - this.#detailAt > 500) {
			this.#detailAt = performance.now();
			this.applyStatus(m.status as DeviceStatus);
		}
	}

	/**
	 * Take a status report from the worker: the UI's `detail`, and the start-of-playing hook
	 * when the output goes from anything else to 'Playing' (a hold and its release don't
	 * change `output`, so they never count as a start).
	 */
	applyStatus(s: DeviceStatus) {
		const was = String(this.detail.output ?? '');
		this.detail = s;
		if (was !== 'Playing' && String(s.output ?? '') === 'Playing') for (const fn of [...this.#onPlaying]) fn();
	}

	/** Run `fn` whenever this device's own output starts playing; returns the way to stop. */
	onPlaying(fn: () => void): () => void {
		this.#onPlaying.add(fn);
		return () => void this.#onPlaying.delete(fn);
	}
}

export const device = new DevicePlayer();
