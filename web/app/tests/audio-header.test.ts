// An original download's rate and depth, read from the file's own first bytes (FLAC
// STREAMINFO, WAV `fmt `). The bytes are untrusted: malformed, truncated and random input
// gives null, never a throw or a silly number.
import { HEADER_BYTES, HeadCollector, audioHeader, storedHeader } from '#lib/offline/audio-header';

/** "fLaC" and a STREAMINFO block for `rate` Hz, `bits` bits, stereo. */
function flac(rate: number, bits: number, last = true): Uint8Array {
	const b = new Uint8Array(8 + 34);
	b.set([0x66, 0x4c, 0x61, 0x43]);
	b[4] = last ? 0x80 : 0x00;
	b[7] = 34;
	const s = 8;
	b.set([0x10, 0x00, 0x10, 0x00], s); // block sizes
	const ch = 2 - 1;
	const bps = bits - 1;
	b[s + 10] = (rate >> 12) & 0xff;
	b[s + 11] = (rate >> 4) & 0xff;
	b[s + 12] = ((rate & 0x0f) << 4) | (ch << 1) | (bps >> 4);
	b[s + 13] = (bps & 0x0f) << 4;
	return b;
}

/** A RIFF WAVE header with `extra` chunks before `fmt `. */
function wav(rate: number, bits: number, extra: { id: string; size: number }[] = []): Uint8Array {
	const parts: number[] = [...'RIFF'].map((c) => c.charCodeAt(0));
	parts.push(0, 0, 0, 0, ...[...'WAVE'].map((c) => c.charCodeAt(0)));
	for (const e of extra) {
		parts.push(...[...e.id].map((c) => c.charCodeAt(0)), e.size & 0xff, (e.size >> 8) & 0xff, 0, 0, ...new Array(e.size + (e.size & 1)).fill(0x41));
	}
	const fmt = new Uint8Array(24);
	const v = new DataView(fmt.buffer);
	fmt.set([...'fmt '].map((c) => c.charCodeAt(0)));
	v.setUint32(4, 16, true);
	v.setUint16(8, 1, true);
	v.setUint16(10, 2, true);
	v.setUint32(12, rate, true);
	v.setUint32(16, (rate * bits) / 4, true);
	v.setUint16(20, bits / 4, true);
	v.setUint16(22, bits, true);
	return new Uint8Array([...parts, ...fmt]);
}

test('FLAC: STREAMINFO gives the rate and depth', () => {
	expect(audioHeader(flac(96_000, 24))).toEqual({ rate: 96_000, bits: 24 });
	expect(audioHeader(flac(44_100, 16))).toEqual({ rate: 44_100, bits: 16 });
	expect(audioHeader(flac(192_000, 24, false))).toEqual({ rate: 192_000, bits: 24 });
	expect(audioHeader(flac(352_800, 32))).toEqual({ rate: 352_800, bits: 32 });
});

test('FLAC after an ID3v2 tag, with and without a footer', () => {
	const tag = (size: number, footer = false) => {
		const t = new Uint8Array(10 + size + (footer ? 10 : 0));
		t.set([0x49, 0x44, 0x33, 4, 0, footer ? 0x10 : 0, (size >> 21) & 0x7f, (size >> 14) & 0x7f, (size >> 7) & 0x7f, size & 0x7f]);
		return t;
	};
	expect(audioHeader(new Uint8Array([...tag(300), ...flac(88_200, 24)]))).toEqual({ rate: 88_200, bits: 24 });
	expect(audioHeader(new Uint8Array([...tag(20, true), ...flac(48_000, 16)]))).toEqual({ rate: 48_000, bits: 16 });
	// A tag bigger than what is looked at: unknown.
	expect(audioHeader(new Uint8Array([...tag(HEADER_BYTES), ...flac(96_000, 24)]))).toBeNull();
	// A size byte with its top bit set is not a valid tag.
	const bad = new Uint8Array([...tag(10), ...flac(96_000, 24)]);
	bad[9] = 0x8a;
	expect(audioHeader(bad)).toBeNull();
});

test('WAV: the fmt chunk, after any other chunks', () => {
	expect(audioHeader(wav(44_100, 16))).toEqual({ rate: 44_100, bits: 16 });
	expect(audioHeader(wav(96_000, 24, [{ id: 'LIST', size: 33 }, { id: 'bext', size: 10 }]))).toEqual({ rate: 96_000, bits: 24 });
});

test('not FLAC or WAV, truncated, or not sane: unknown', () => {
	const ogg = new TextEncoder().encode('OggS\0\x02rest of a page');
	const mp3 = new Uint8Array([0xff, 0xfb, 0x90, 0x64, 0, 0, 0, 0]);
	for (const b of [new Uint8Array(0), ogg, mp3, flac(96_000, 24).subarray(0, 30), wav(44_100, 16).subarray(0, 30), new TextEncoder().encode('<html>')]) {
		expect(audioHeader(b)).toBeNull();
	}
	// The first block isn't STREAMINFO.
	const notInfo = flac(96_000, 24);
	notInfo[4] = 0x84;
	expect(audioHeader(notInfo)).toBeNull();
	// A rate of 0 or below 1 kHz.
	expect(audioHeader(flac(0, 16))).toBeNull();
	expect(audioHeader(flac(500, 16))).toBeNull();
	expect(audioHeader(wav(44_100, 0))).toBeNull();
	// A chunk size that runs off the end.
	expect(audioHeader(wav(44_100, 16, [{ id: 'junk', size: 0xffff }]).subarray(0, 64))).toBeNull();
	for (const v of [null, undefined, 'fLaC', [0x66, 0x4c, 0x61, 0x43], new ArrayBuffer(42)]) expect(audioHeader(v as unknown as Uint8Array)).toBeNull();
});

test('robustness: random and mutated bytes never throw, and anything found is sane', () => {
	let seed = 99;
	const rnd = () => (seed = (seed * 1103515245 + 12345) % 2 ** 31) / 2 ** 31;
	const seeds = [flac(96_000, 24), wav(48_000, 24, [{ id: 'LIST', size: 7 }]), new Uint8Array([0x49, 0x44, 0x33, 4, 0, 0, 0, 0, 0, 5, 1, 2, 3, 4, 5, ...flac(44_100, 16)])];
	for (let i = 0; i < 5000; i++) {
		let b: Uint8Array;
		if (i % 4 === 0) {
			b = new Uint8Array(Math.floor(rnd() * 200));
			for (let j = 0; j < b.length; j++) b[j] = Math.floor(rnd() * 256);
		} else {
			b = seeds[i % seeds.length]!.slice();
			for (let n = 1 + Math.floor(rnd() * 4); n > 0; n--) b[Math.floor(rnd() * b.length)] = Math.floor(rnd() * 256);
			if (rnd() < 0.3) b = b.subarray(0, Math.floor(rnd() * b.length));
		}
		const r = audioHeader(b);
		if (r) {
			expect(Number.isSafeInteger(r.rate) && r.rate >= 1000 && r.rate <= 1_536_000).toBe(true);
			expect(Number.isSafeInteger(r.bits) && r.bits >= 1 && r.bits <= 32).toBe(true);
		}
	}
});

test('the collector keeps only the first bytes, across pieces', () => {
	const c = new HeadCollector();
	const piece = new Uint8Array(3000).fill(7);
	c.add(flac(96_000, 24));
	c.add(piece);
	c.add(piece);
	expect(c.bytes.length).toBe(HEADER_BYTES);
	expect(audioHeader(c.bytes)).toEqual({ rate: 96_000, bits: 24 });
	// A copy: changing the source afterwards changes nothing.
	piece.fill(0);
	expect(c.bytes[100]).toBe(7);
});

test('stored numbers are checked when read back', () => {
	expect(storedHeader({ rate: 96_000, bits: 24 })).toEqual({ rate: 96_000, bits: 24 });
	expect(storedHeader({ rate: 96_000 })).toEqual({ rate: 96_000 });
	for (const t of [null, undefined, {}, { rate: '96000', bits: '24' }, { rate: -1, bits: 0 }, { rate: 1e9, bits: 33 }, { rate: NaN, bits: 1.5 }]) {
		expect(storedHeader(t as never)).toEqual({});
	}
});
