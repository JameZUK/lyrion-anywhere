// One visualiser frame from the player worker (docs/plans/wave2-design.md §4.1): the core's
// analysis as a flat array (see wasm/src/lib.rs, WasmVisualiser::take). Anything else, or
// values out of range, is cleaned here, so drawing never sees NaN or a runaway level.

export const BANDS = 32;
export const SCOPE_POINTS = 128;
export const FRAME_LEN = BANDS + SCOPE_POINTS + 5;
/** The quietest level reported (dBFS). */
export const FLOOR_DB = -90;

export interface VisualFrame {
	/** 32 log-spaced bands, 40 Hz–16 kHz, each 0–1. */
	bands: Float32Array;
	/** 128 samples, −1–1, oldest first. */
	scope: Float32Array;
	/** Left and right, dBFS (FLOOR_DB–0). */
	rmsDb: [number, number];
	peakDb: [number, number];
	/** Bass energy, 0–1. */
	bass: number;
}

const unit = (x: number) => (Number.isFinite(x) ? Math.min(1, Math.max(0, x)) : 0);
const signed = (x: number) => (Number.isFinite(x) ? Math.min(1, Math.max(-1, x)) : 0);
const dbfs = (x: number) => (Number.isFinite(x) ? Math.min(0, Math.max(FLOOR_DB, x)) : FLOOR_DB);

/** A frame from the worker's message, or null for anything that isn't one. */
export function parseFrame(v: unknown): VisualFrame | null {
	if (!(v instanceof Float32Array) || v.length !== FRAME_LEN) return null;
	const bands = v.slice(0, BANDS).map(unit);
	const scope = v.slice(BANDS, BANDS + SCOPE_POINTS).map(signed);
	const at = BANDS + SCOPE_POINTS;
	return {
		bands,
		scope,
		rmsDb: [dbfs(v[at]!), dbfs(v[at + 1]!)],
		peakDb: [dbfs(v[at + 2]!), dbfs(v[at + 3]!)],
		bass: unit(v[at + 4]!),
	};
}

/** A silent frame (what is drawn before the first one arrives, and when the music stops). */
export function silentFrame(): VisualFrame {
	return {
		bands: new Float32Array(BANDS),
		scope: new Float32Array(SCOPE_POINTS),
		rmsDb: [FLOOR_DB, FLOOR_DB],
		peakDb: [FLOOR_DB, FLOOR_DB],
		bass: 0,
	};
}
