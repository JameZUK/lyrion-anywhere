// VU meter ballistics, drawn by the client (docs/plans/wave2-design.md §4.1): the needle
// follows the RMS level like a classic VU meter, reaching 99% of a step in about 300 ms on
// the way up and on the way down, and a peak marker holds the loudest peak for a moment
// before it falls back.
import { FLOOR_DB } from './frame';

/** The needle reaches 99% of a step in this long. */
export const VU_RESPONSE_MS = 300;
/** Time constant of that response (99% after 4.6 time constants). */
export const VU_TAU_MS = VU_RESPONSE_MS / Math.log(100);
/** The peak marker holds this long, then falls at PEAK_FALL_DB_PER_S. */
export const PEAK_HOLD_MS = 1500;
export const PEAK_FALL_DB_PER_S = 20;
/** Longest step taken at once (after the page was hidden, say): the needle just settles. */
const MAX_STEP_MS = 1000;

/** 0 VU on the scale is this many dBFS (the usual digital reference). */
export const VU_REFERENCE_DBFS = -18;
/** The scale runs from −20 VU to +3 VU. */
export const VU_MIN = -20;
export const VU_MAX = 3;

const clean = (db: number) => (Number.isFinite(db) ? Math.min(0, Math.max(FLOOR_DB, db)) : FLOOR_DB);

export class VuBallistics {
	/** The needle's level (dBFS). */
	level = FLOOR_DB;
	/** The held peak (dBFS). */
	peak = FLOOR_DB;
	#peakAt = -Infinity;

	/**
	 * Move on by `dtMs` towards `rmsDb`, noting `peakDb`; `nowMs` times the peak hold.
	 * Returns the needle level.
	 */
	step(rmsDb: number, peakDb: number, dtMs: number, nowMs: number): number {
		const dt = Number.isFinite(dtMs) ? Math.min(MAX_STEP_MS, Math.max(0, dtMs)) : 0;
		const target = clean(rmsDb);
		const k = 1 - Math.exp(-dt / VU_TAU_MS);
		this.level = clean(this.level + (target - this.level) * k);
		const p = clean(peakDb);
		if (p >= this.peak) {
			this.peak = p;
			this.#peakAt = nowMs;
		} else if (nowMs - this.#peakAt > PEAK_HOLD_MS) {
			this.peak = clean(this.peak - (PEAK_FALL_DB_PER_S * dt) / 1000);
		}
		// The marker never sits below the needle.
		this.peak = Math.max(this.peak, this.level);
		return this.level;
	}

	reset() {
		this.level = FLOOR_DB;
		this.peak = FLOOR_DB;
		this.#peakAt = -Infinity;
	}
}

/** Where a level (dBFS) sits on the VU scale, 0 (−20 VU) to 1 (+3 VU). */
export function needlePosition(db: number): number {
	const vu = clean(db) - VU_REFERENCE_DBFS;
	return Math.min(1, Math.max(0, (vu - VU_MIN) / (VU_MAX - VU_MIN)));
}

/** Bars fall back at this rate (full height per second) and jump up at once. */
export const BAR_FALL_PER_S = 1.6;

/** Bars that rise at once and fall smoothly, so the spectrum doesn't flicker. */
export function fallBars(shown: Float32Array, target: Float32Array, dtMs: number): Float32Array {
	const dt = Number.isFinite(dtMs) ? Math.min(MAX_STEP_MS, Math.max(0, dtMs)) : 0;
	const fall = (BAR_FALL_PER_S * dt) / 1000;
	const n = Math.min(shown.length, target.length);
	for (let i = 0; i < n; i++) {
		const t = Number.isFinite(target[i]!) ? target[i]! : 0;
		shown[i] = Math.max(t, shown[i]! - fall, 0);
	}
	return shown;
}
