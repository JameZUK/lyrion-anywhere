// Volume on "this device".
//
// LMS sends its volume to the player as linear gains (`audg`, already shaped by LMS's own
// volume curve). The browser player applies them at its output, after the audio it has
// queued, so a change is heard at once:
// - Web Audio: a ramped gain in the AudioWorklet;
// - media element: the element's `volume`.
// iPhone and iPad don't let a page set a media element's volume (it always reads 1), and
// only the media element keeps playing in the background there. Such a device plays at
// full scale, its own volume buttons set the level, and LMS's volume is held at 100%.
import type { PlaybackMode } from './mode';

/** The user's choice, kept per browser. */
export type VolumeSetting = 'auto' | 'follow' | 'device';

export const VOLUME_LABELS: Record<VolumeSetting, string> = {
	auto: 'Automatic',
	follow: 'Follow LMS volume',
	device: 'Device buttons only',
};

/** What the browser can do, for the output mode in use. */
export interface VolumeAbility {
	mode: PlaybackMode;
	/** A media element's `volume` can be set (false on iPhone/iPad). */
	mediaVolume: boolean;
}

export interface VolumePlan {
	/** Apply LMS's volume here. If false, play at full scale and hold LMS at 100%. */
	follow: boolean;
	/** Where the volume is applied: 'gain' (Web Audio), 'element', or 'none'. */
	apply: 'gain' | 'element' | 'none';
	/** Why, for Settings. */
	reason: string;
}

/** Shown instead of the volume slider when the device's own buttons set the level. */
export const DEVICE_BUTTONS_NOTE = "Use this device's volume buttons";

const NO_ELEMENT_VOLUME =
	"This browser doesn't let a web page set the volume of audio that keeps playing in the background (iPhone and iPad do this), so this device's volume buttons set the level.";

/** Can this device follow LMS's volume? If not, why not. */
export function canFollow(a: VolumeAbility): { ok: true } | { ok: false; why: string } {
	if (a.mode === 'worklet' || a.mediaVolume) return { ok: true };
	return { ok: false, why: NO_ELEMENT_VOLUME };
}

/** The behaviour for an ability and a setting. Pure: tested on its own. */
export function planVolume(a: VolumeAbility, setting: VolumeSetting): VolumePlan {
	const follow = (reason: string): VolumePlan => ({ follow: true, apply: a.mode === 'worklet' ? 'gain' : 'element', reason });
	const buttons = (reason: string): VolumePlan => ({ follow: false, apply: 'none', reason });
	const can = canFollow(a);
	if (setting === 'device') return buttons('Chosen in Settings: LMS volume stays at 100%, and this device’s volume buttons set the level.');
	if (setting === 'follow') {
		return can.ok ? follow('Chosen in Settings: LMS volume is applied on this device.') : buttons(`Follow LMS volume isn't possible here. ${can.why}`);
	}
	return can.ok
		? follow('LMS volume is applied on this device, so any controller can change it.')
		: buttons(can.why);
}

/**
 * Whether a media element's volume can be set: a feature test, not a user-agent check.
 * On iPhone/iPad the assignment is ignored and the volume reads back as 1.
 */
export function detectMediaVolume(make: () => HTMLMediaElement = () => new Audio()): boolean {
	try {
		const a = make();
		a.volume = 0.5;
		const ok = Math.abs(a.volume - 0.5) < 0.01;
		a.volume = 1;
		return ok;
	} catch {
		return false;
	}
}

/** A usable gain: finite, 0..1. LMS never amplifies, and neither do we. */
export function clampGain(g: unknown): number {
	return typeof g === 'number' && Number.isFinite(g) ? Math.min(1, Math.max(0, g)) : 1;
}

/** LMS's [left, right] gains as checked numbers (anything unusable reads as full scale). */
export function cleanGains(v: unknown): [number, number] {
	if (!Array.isArray(v) || v.length < 1) return [1, 1];
	const l = clampGain(v[0]);
	return [l, v.length > 1 ? clampGain(v[1]) : l];
}

/** The gains to apply for a plan: LMS's when following, otherwise full scale. */
export function outputGains(plan: VolumePlan | null, lms: [number, number]): [number, number] {
	return plan && plan.follow ? lms : [1, 1];
}

/** A media element has one volume: balance can't be applied, so the louder side wins. */
export function elementVolume(g: [number, number]): number {
	return Math.max(clampGain(g[0]), clampGain(g[1]));
}

/** LMS's volume reads as anything but full scale (as far as gains can tell). */
export function belowFull(g: [number, number]): boolean {
	return g[0] < 0.999 || g[1] < 0.999;
}

/** Volume values for a ramp from `from` to `to` in `steps` steps (the last is `to`). */
export function rampSteps(from: number, to: number, steps: number): number[] {
	const n = Math.max(1, Math.floor(steps));
	const out: number[] = [];
	for (let i = 1; i <= n; i++) out.push(from + ((to - from) * i) / n);
	out[n - 1] = to;
	return out;
}

export interface Timers {
	setTimeout: (fn: () => void, ms: number) => unknown;
	clearTimeout: (id: unknown) => void;
}

/** Ramp length and step for media element volume changes (to avoid clicks). */
export const ELEMENT_RAMP_MS = 30;
const ELEMENT_RAMP_STEP_MS = 5;

/**
 * Sets a media element's volume, ramped over a few milliseconds. The first step applies at
 * once; a new value replaces a ramp still running.
 */
export class ElementVolume {
	#timer: unknown = null;
	#timers: Timers;

	constructor(
		private el: { volume: number },
		timers: Timers = { setTimeout: (f, ms) => setTimeout(f, ms), clearTimeout: (id) => clearTimeout(id as ReturnType<typeof setTimeout>) },
	) {
		this.#timers = timers;
	}

	set(target: number) {
		const to = clampGain(target);
		if (this.#timer !== null) this.#timers.clearTimeout(this.#timer);
		this.#timer = null;
		const from = clampGain(this.el.volume);
		if (Math.abs(from - to) < 1e-4) {
			this.#write(to);
			return;
		}
		const steps = rampSteps(from, to, ELEMENT_RAMP_MS / ELEMENT_RAMP_STEP_MS);
		const next = () => {
			this.#write(steps.shift()!);
			this.#timer = steps.length ? this.#timers.setTimeout(next, ELEMENT_RAMP_STEP_MS) : null;
		};
		next();
	}

	cancel() {
		if (this.#timer !== null) this.#timers.clearTimeout(this.#timer);
		this.#timer = null;
	}

	#write(v: number) {
		try {
			this.el.volume = v;
		} catch {
			/* read-only here: nothing to do */
		}
	}
}

const KEY = 'anywhere.device.volume';

export function loadVolumeSetting(storage: Pick<Storage, 'getItem'> | undefined = safeLocal()): VolumeSetting {
	try {
		const v = storage?.getItem(KEY);
		return v === 'follow' || v === 'device' ? v : 'auto';
	} catch {
		return 'auto';
	}
}

export function saveVolumeSetting(s: VolumeSetting, storage: Pick<Storage, 'setItem' | 'removeItem'> | undefined = safeLocal()) {
	try {
		if (s === 'auto') storage?.removeItem(KEY);
		else storage?.setItem(KEY, s);
	} catch {
		/* not essential */
	}
}

// "Fix this player's output level at 100%": LMS's per-player Volume Control preference
// (`digitalVolumeControl`; 1, LMS's default, lets LMS set the volume; 0 is "Output level is
// fixed at 100%"). At 0, LMS sends the player full scale, its own interfaces hide that
// player's volume, and this device's own volume sets the level, so no controller can turn it
// down. On by default, per browser; it applies to this device's own Lyrion player only.

/** Shown instead of the volume slider for any player whose output level LMS keeps at 100%. */
export const FIXED_LEVEL_NOTE = 'Output level is fixed at 100%';
export const FIXED_LEVEL_LABEL = 'Fix this player’s output level at 100%';
export const FIXED_LEVEL_EXPLANATION =
	'Sets this player’s Volume Control in Lyrion to “Output level is fixed at 100%”, so this device’s own volume sets the level and no controller can turn it down.';
/** Why "Volume on this device" can't be chosen while the level is fixed. */
export const FIXED_LEVEL_NO_FOLLOW = 'Not available while the output level is fixed: there is no LMS volume to follow.';

/** A player's status says LMS keeps its output level at 100% (Volume Control off). */
export function levelFixed(status: { digital_volume_control?: unknown } | null | undefined): boolean {
	const v = status?.digital_volume_control;
	return v === 0 || v === '0';
}

/** LMS's Volume Control as reported (1 on, 0 fixed), or null when unknown. */
function reportedControl(v: unknown): 0 | 1 | null {
	if (v === 0 || v === '0') return 0;
	if (v === 1 || v === '1') return 1;
	return null;
}

/**
 * What to set `digitalVolumeControl` to, for the setting and what LMS reports: '0' to fix
 * the level, '1' to give LMS the volume back, or null when they agree or LMS's value is
 * unknown (nothing is sent on a guess).
 */
export function fixedLevelChange(fixed: boolean, reported: unknown): '0' | '1' | null {
	const r = reportedControl(reported);
	if (r === null) return null;
	if (fixed && r === 1) return '0';
	if (!fixed && r === 0) return '1';
	return null;
}

const FIXED_KEY = 'anywhere.device.fixedLevel';

/** On unless turned off in this browser. */
export function loadFixedLevel(storage: Pick<Storage, 'getItem'> | undefined = safeLocal()): boolean {
	try {
		return storage?.getItem(FIXED_KEY) !== 'off';
	} catch {
		return true;
	}
}

/** Only the off state is stored: the default leaves no key behind. */
export function saveFixedLevel(on: boolean, storage: Pick<Storage, 'setItem' | 'removeItem'> | undefined = safeLocal()) {
	try {
		if (on) storage?.removeItem(FIXED_KEY);
		else storage?.setItem(FIXED_KEY, 'off');
	} catch {
		/* not essential */
	}
}

function safeLocal(): Storage | undefined {
	try {
		return globalThis.localStorage;
	} catch {
		return undefined;
	}
}
