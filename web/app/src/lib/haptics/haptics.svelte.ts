// Haptics (wave2-design.md §5.4): light ticks through `navigator.vibrate`, where the browser
// has it (Android browsers; not iOS or most desktops). Off unless switched on in Settings;
// the choice is kept in this browser only. Nothing happens when the API is missing.

/** What ticks: a skip by swipe, scrub detents (every 10 %), volume steps, drag pick-up and
 * drop, setting a rating or a sleep timer, and a long press (car mode's stop, Play on…). */
export type HapticEvent = 'skip' | 'scrub' | 'volume' | 'pickup' | 'drop' | 'rating' | 'sleep' | 'press';

/** Short and light: milliseconds of vibration (a pattern for the sleep timer's double tick). */
export const PATTERNS: Record<HapticEvent, number | number[]> = {
	skip: 12,
	scrub: 6,
	volume: 6,
	pickup: 15,
	drop: 10,
	rating: 12,
	sleep: [10, 60, 10],
	press: 18,
};

export const HAPTICS_KEY = 'anywhere.haptics';
/** Volume ticks every this many steps (of 100). */
export const VOLUME_STEP = 5;
/** Scrubbing ticks at each tenth of the track. */
export const SCRUB_DETENTS = 10;

type Vibrate = (pattern: number | number[]) => boolean;

function vibrator(): Vibrate | null {
	const n = typeof navigator !== 'undefined' ? (navigator as Navigator & { vibrate?: Vibrate }) : null;
	return n && typeof n.vibrate === 'function' ? n.vibrate.bind(n) : null;
}

function storage(): Storage | undefined {
	try {
		return globalThis.localStorage;
	} catch {
		return undefined;
	}
}

/** Did a value move from one step to another (of size `step`)? */
export function crossed(prev: number, next: number, step: number): boolean {
	if (!(step > 0) || !Number.isFinite(prev) || !Number.isFinite(next)) return false;
	return Math.floor(prev / step) !== Math.floor(next / step);
}

class Haptics {
	enabled = $state(false);

	constructor() {
		this.reload();
	}

	/** Whether this browser can vibrate at all. */
	get supported(): boolean {
		return vibrator() !== null;
	}

	/** Read the stored choice again (default: off). */
	reload() {
		try {
			this.enabled = storage()?.getItem(HAPTICS_KEY) === '1';
		} catch {
			this.enabled = false;
		}
	}

	set(on: boolean) {
		this.enabled = !!on;
		try {
			storage()?.setItem(HAPTICS_KEY, on ? '1' : '0');
		} catch {
			/* storage blocked: the choice lasts until the page closes */
		}
	}

	/** A light tick for `event`, if switched on and the browser can. */
	tick(event: HapticEvent): boolean {
		if (!this.enabled) return false;
		const v = vibrator();
		if (!v) return false;
		try {
			return v(PATTERNS[event]);
		} catch {
			return false;
		}
	}

	/** Volume moved: a tick at each step of `VOLUME_STEP`. */
	volume(prev: number, next: number) {
		if (crossed(prev, next, VOLUME_STEP)) this.tick('volume');
	}

	/** Scrubbing moved: a tick at each tenth of `total`. */
	scrub(prev: number, next: number, total: number) {
		if (total > 0 && crossed(prev, next, total / SCRUB_DETENTS)) this.tick('scrub');
	}
}

export const haptics = new Haptics();
