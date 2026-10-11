// The gateway's track analysis in its binary form (GET /v1/analysis/track/{id}?format=binary,
// docs/PROTOCOL.md). The bytes come from the network, so the decoder trusts nothing: every
// length and value is checked, anything malformed gives null, and it never throws.
//
// Layout (version 1), little-endian:
//   "LAAN"  u16 version  u32 rate  u16 channels  u64 frames
//   f32 ×6  integrated LUFS, LRA, max momentary, max short-term, true peak dBTP, sample peak
//           (NaN = none)
//   f32 ×2  leading, trailing silence (s)
//   edge ×2 f32 slope, f32 start LUFS (NaN = none), f32 end LUFS (NaN = none), f32 length, u8 fade
//   u64     invalid samples
//   u16 N   u8 ×N peak  u8 ×N RMS

export const ANALYSIS_VERSION = 1;
export const MAX_BUCKETS = 10_000;
/** Largest encoding accepted (as the gateway's own decoder). */
export const MAX_BYTES = 64 + 22 * 4 + 2 * 17 + 2 * MAX_BUCKETS + 64;
export const BINARY_TYPE = 'application/vnd.lyrion-anywhere.analysis';

export interface Edge {
	slopeDbPerS: number;
	startLufs: number | null;
	endLufs: number | null;
	lengthS: number;
	fade: boolean;
}

export interface TrackAnalysis {
	sampleRate: number;
	channels: number;
	/** Frames analysed (a Number: exact up to 2^53, far beyond any track). */
	frames: number;
	durationS: number;
	integratedLufs: number | null;
	loudnessRangeLu: number | null;
	maxMomentaryLufs: number | null;
	maxShortTermLufs: number | null;
	truePeakDbtp: number | null;
	samplePeakDbfs: number | null;
	leadingSilenceS: number;
	trailingSilenceS: number;
	fadeIn: Edge;
	fadeOut: Edge;
	invalidSamples: number;
	/** Waveform buckets, 0 = silence, 255 = full scale. Same length. */
	peak: Uint8Array;
	rms: Uint8Array;
}

class Truncated extends Error {}
class Bad extends Error {}

class Reader {
	#v: DataView;
	#o = 0;
	constructor(buf: ArrayBuffer | Uint8Array) {
		this.#v =
			buf instanceof Uint8Array ? new DataView(buf.buffer, buf.byteOffset, buf.byteLength) : new DataView(buf);
	}
	get left() {
		return this.#v.byteLength - this.#o;
	}
	#need(n: number) {
		if (this.left < n) throw new Truncated();
		const o = this.#o;
		this.#o += n;
		return o;
	}
	u8() {
		return this.#v.getUint8(this.#need(1));
	}
	u16() {
		return this.#v.getUint16(this.#need(2), true);
	}
	u32() {
		return this.#v.getUint32(this.#need(4), true);
	}
	u64() {
		const o = this.#need(8);
		const lo = this.#v.getUint32(o, true);
		const hi = this.#v.getUint32(o + 4, true);
		// Above 2^53 can't be a real count: refuse rather than round.
		if (hi > 0x1fffff) throw new Bad();
		return hi * 2 ** 32 + lo;
	}
	/** A finite value within ±1e6. */
	f32() {
		const v = this.#v.getFloat32(this.#need(4), true);
		if (!Number.isFinite(v) || Math.abs(v) > 1e6) throw new Bad();
		return v;
	}
	/** NaN means "none"; infinities and huge values are refused. */
	opt(): number | null {
		const v = this.#v.getFloat32(this.#need(4), true);
		if (Number.isNaN(v)) return null;
		if (!Number.isFinite(v) || Math.abs(v) > 1e6) throw new Bad();
		return v;
	}
	bytes(n: number) {
		const o = this.#need(n);
		return new Uint8Array(this.#v.buffer.slice(this.#v.byteOffset + o, this.#v.byteOffset + o + n));
	}
	edge(): Edge {
		const slopeDbPerS = this.f32();
		const startLufs = this.opt();
		const endLufs = this.opt();
		const lengthS = this.f32();
		const flag = this.u8();
		if (flag > 1) throw new Bad();
		return { slopeDbPerS, startLufs, endLufs, lengthS, fade: flag === 1 };
	}
}

const round2 = (x: number) => Math.round(x * 100) / 100;
const r2 = (x: number | null) => (x === null ? null : round2(x));

/** Decode the binary form; null for anything malformed, oversized or of another version. */
export function decodeAnalysis(buf: ArrayBuffer | Uint8Array): TrackAnalysis | null {
	try {
		if (buf.byteLength > MAX_BYTES) return null;
		const r = new Reader(buf);
		if (r.u8() !== 0x4c || r.u8() !== 0x41 || r.u8() !== 0x41 || r.u8() !== 0x4e) return null; // "LAAN"
		if (r.u16() !== ANALYSIS_VERSION) return null;
		const sampleRate = r.u32();
		const channels = r.u16();
		const frames = r.u64();
		const integratedLufs = r2(r.opt());
		const loudnessRangeLu = r2(r.opt());
		const maxMomentaryLufs = r2(r.opt());
		const maxShortTermLufs = r2(r.opt());
		const truePeakDbtp = r2(r.opt());
		const samplePeakDbfs = r2(r.opt());
		const leadingSilenceS = round2(r.f32());
		const trailingSilenceS = round2(r.f32());
		const fadeIn = r.edge();
		const fadeOut = r.edge();
		const invalidSamples = r.u64();
		const n = r.u16();
		if (n > MAX_BUCKETS) return null;
		const peak = r.bytes(n);
		const rms = r.bytes(n);
		if (r.left !== 0) return null;
		return {
			sampleRate,
			channels,
			frames,
			durationS: sampleRate > 0 ? frames / sampleRate : 0,
			integratedLufs,
			loudnessRangeLu,
			maxMomentaryLufs,
			maxShortTermLufs,
			truePeakDbtp,
			samplePeakDbfs,
			leadingSilenceS,
			trailingSilenceS,
			fadeIn,
			fadeOut,
			invalidSamples,
			peak,
			rms,
		};
	} catch {
		return null;
	}
}
