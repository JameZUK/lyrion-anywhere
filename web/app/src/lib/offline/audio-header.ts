// The sample rate and bit depth of a downloaded original, read from the start of the file
// itself (the server's track details don't carry them), so a 24/96 FLAC kept on this device
// is badged Hi-Res like the same file streamed. FLAC (STREAMINFO, after an optional ID3v2
// tag) and WAV (RIFF `fmt `) only; anything else is unknown. The bytes are untrusted: every
// read is bounds-checked, and nothing here throws.

/** How much of the start of a file is looked at. */
export const HEADER_BYTES = 4096;

export interface AudioHeader {
	rate: number;
	bits: number;
}

const sane = (rate: number, bits: number): AudioHeader | null =>
	Number.isSafeInteger(rate) && rate >= 1000 && rate <= 1_536_000 && Number.isSafeInteger(bits) && bits >= 1 && bits <= 32 ? { rate, bits } : null;

const tag = (b: Uint8Array, at: number, s: string) => at >= 0 && at + s.length <= b.length && [...s].every((c, i) => b[at + i] === c.charCodeAt(0));

function flac(b: Uint8Array, at: number): AudioHeader | null {
	// "fLaC", then the first metadata block, which must be STREAMINFO (type 0, 34 bytes).
	if (!tag(b, at, 'fLaC') || at + 8 + 34 > b.length) return null;
	if ((b[at + 4]! & 0x7f) !== 0) return null;
	const len = (b[at + 5]! << 16) | (b[at + 6]! << 8) | b[at + 7]!;
	if (len < 34) return null;
	const s = at + 8;
	// After the block sizes (2 + 2) and frame sizes (3 + 3): 20 bits of rate, 3 of channels
	// less one, 5 of bits per sample less one.
	const rate = (b[s + 10]! << 12) | (b[s + 11]! << 4) | (b[s + 12]! >> 4);
	const bits = (((b[s + 12]! & 0x01) << 4) | (b[s + 13]! >> 4)) + 1;
	return sane(rate, bits);
}

function wav(b: Uint8Array): AudioHeader | null {
	if (!tag(b, 0, 'RIFF') || !tag(b, 8, 'WAVE')) return null;
	const v = new DataView(b.buffer, b.byteOffset, b.byteLength);
	let at = 12;
	// Walk the chunks to `fmt ` (a bounded number of them, within what was read).
	for (let n = 0; n < 64 && at + 8 <= b.length; n++) {
		const size = v.getUint32(at + 4, true);
		if (tag(b, at, 'fmt ')) {
			if (size < 16 || at + 8 + 16 > b.length) return null;
			return sane(v.getUint32(at + 12, true), v.getUint16(at + 22, true));
		}
		// Chunks are padded to an even size.
		at += 8 + size + (size & 1);
	}
	return null;
}

/** The rate and depth stated at the start of `head`, or null when not FLAC or WAV, or not sane. */
export function audioHeader(head: Uint8Array): AudioHeader | null {
	if (!(head instanceof Uint8Array)) return null;
	const b = head.length > HEADER_BYTES ? head.subarray(0, HEADER_BYTES) : head;
	if (tag(b, 0, 'ID3') && b.length >= 10) {
		// An ID3v2 tag before the FLAC stream: its size is four 7-bit bytes, plus a footer if flagged.
		if ((b[6]! | b[7]! | b[8]! | b[9]!) & 0x80) return null;
		const size = (b[6]! << 21) | (b[7]! << 14) | (b[8]! << 7) | b[9]!;
		return flac(b, 10 + size + (b[5]! & 0x10 ? 10 : 0));
	}
	return flac(b, 0) ?? wav(b);
}

/** Bytes collected from the start of a download, up to {@link HEADER_BYTES}. */
export class HeadCollector {
	#parts: Uint8Array[] = [];
	#len = 0;

	add(bytes: Uint8Array) {
		if (this.#len >= HEADER_BYTES) return;
		const take = bytes.subarray(0, HEADER_BYTES - this.#len);
		this.#parts.push(take.slice());
		this.#len += take.length;
	}

	get bytes(): Uint8Array {
		const out = new Uint8Array(this.#len);
		let at = 0;
		for (const p of this.#parts) {
			out.set(p, at);
			at += p.length;
		}
		return out;
	}
}

/** The stored rate and depth of a download, checked (records come from IndexedDB). */
export function storedHeader(t: { rate?: unknown; bits?: unknown } | null | undefined): Partial<AudioHeader> {
	const rate = t?.rate;
	const bits = t?.bits;
	return {
		...(typeof rate === 'number' && Number.isSafeInteger(rate) && rate >= 1000 && rate <= 1_536_000 ? { rate } : {}),
		...(typeof bits === 'number' && Number.isSafeInteger(bits) && bits >= 1 && bits <= 32 ? { bits } : {}),
	};
}
