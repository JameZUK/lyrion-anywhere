// When the audio is heard, for keeping this device in step with LMS's other players.
//
// - Web Audio: `AudioContext.getOutputTimestamp()` says which context time is being heard
//   at which `performance.now()`, including the output's latency. Where it is missing (or
//   not running yet), `currentTime` less `outputLatency` and `baseLatency` is the next best.
//   The player worker turns this into the frames that are audible (see
//   `static/player/presentation.js`).
// - The media element (iOS): `currentTime` is the only clue. It moves in small, uneven steps,
//   so it is smoothed against the wall clock, which it should track one for one.
//
// Times passed to the worker are absolute (`performance.timeOrigin + performance.now()`, in
// ms): the page and the worker each have their own `performance.now()`.

/** The parts of an AudioContext used here. */
export interface ClockSource {
	readonly currentTime: number;
	readonly sampleRate: number;
	readonly outputLatency?: number;
	readonly baseLatency?: number;
	getOutputTimestamp?: () => { contextTime?: number; performanceTime?: number };
}

export interface OutputClock {
	/** Context time (s) heard at `at`. */
	ctx: number;
	/** Absolute time (ms). */
	at: number;
	rate: number;
}

/** A timestamp further than this from now (ms) is not believed. */
const MAX_TIMESTAMP_SKEW_MS = 1000;
/** Latency figures beyond this (s) are not believed. */
const MAX_LATENCY_S = 2;

const finite = (n: unknown): n is number => typeof n === 'number' && Number.isFinite(n);

/** Which context time is heard when, or null if the context can't say yet. */
export function outputClock(ctx: ClockSource, timeOrigin: number, perfNow: number): OutputClock | null {
	const rate = ctx.sampleRate;
	if (!finite(rate) || rate <= 0) return null;
	let ts: { contextTime?: number; performanceTime?: number } | undefined;
	try {
		ts = ctx.getOutputTimestamp?.();
	} catch {
		ts = undefined;
	}
	if (ts && finite(ts.contextTime) && finite(ts.performanceTime) && ts.contextTime > 0 && ts.performanceTime > 0 && Math.abs(perfNow - ts.performanceTime) <= MAX_TIMESTAMP_SKEW_MS) {
		return { ctx: ts.contextTime, at: timeOrigin + ts.performanceTime, rate };
	}
	if (!finite(ctx.currentTime) || ctx.currentTime <= 0) return null;
	const lat = (v: unknown) => (finite(v) && v >= 0 && v <= MAX_LATENCY_S ? v : 0);
	const heard = ctx.currentTime - lat(ctx.outputLatency) - lat(ctx.baseLatency);
	return heard > 0 ? { ctx: heard, at: timeOrigin + perfNow, rate } : null;
}

/** A step of more than this (s) from the smoothed position is a real jump (a seek, a stall). */
const JUMP_S = 0.15;
/** How much of each reading's difference is taken in. */
const GAIN = 0.15;

/**
 * Smooths a media element's play head while it plays: it tracks the offset between the play
 * head and the wall clock (constant while playing at normal speed), moving towards each new
 * reading a little at a time, and starting again on a jump.
 */
export class PlayheadSmoother {
	#offset: number | null = null;

	/** The smoothed position (s) for a reading `raw` taken at wall time `wall` (s). */
	update(raw: number, wall: number): number {
		if (!finite(raw) || !finite(wall)) return raw;
		const o = raw - wall;
		if (this.#offset === null || Math.abs(o - this.#offset) > JUMP_S) this.#offset = o;
		else this.#offset += GAIN * (o - this.#offset);
		return wall + this.#offset;
	}

	/** Paused, stalled or moved: start again from the next reading. */
	reset() {
		this.#offset = null;
	}
}
