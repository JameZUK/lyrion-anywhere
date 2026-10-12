// A small QR decoder for tests, written from ISO/IEC 18004 apart from the encoder it checks
// (web/app/src/lib/qr/qr.js): it finds the format information, rebuilds the function
// pattern map, reads the codewords back in placement order, de-interleaves the blocks,
// checks every block's Reed-Solomon syndromes are zero and parses the byte-mode segment.
// Level M, versions 1 to 10 (what the encoder makes). Throws on anything it can't read.

const BLOCKS_M = {
	1: [10, [[1, 16]]],
	2: [16, [[1, 28]]],
	3: [26, [[1, 44]]],
	4: [18, [[2, 32]]],
	5: [24, [[2, 43]]],
	6: [16, [[4, 27]]],
	7: [18, [[4, 31]]],
	8: [22, [[2, 38], [2, 39]]],
	9: [22, [[3, 36], [2, 37]]],
	10: [26, [[4, 43], [1, 44]]],
};
const ALIGN = { 1: [], 2: [6, 18], 3: [6, 22], 4: [6, 26], 5: [6, 30], 6: [6, 34], 7: [6, 22, 38], 8: [6, 24, 42], 9: [6, 26, 46], 10: [6, 28, 50] };

/** GF(2) remainder of value (with `bits` bits) divided by poly (of degree `deg`). */
function gf2mod(value, poly, deg) {
	for (let i = 31 - Math.clz32(value); i >= deg; i--) if (value & (1 << i)) value ^= poly << (i - deg);
	return value;
}

const popcount = (v) => v.toString(2).replace(/0/g, '').length;

function readFormat(size, get) {
	let a = 0;
	// Copy 1, most significant bit first, around the top-left finder.
	const first = [[0, 8], [1, 8], [2, 8], [3, 8], [4, 8], [5, 8], [7, 8], [8, 8], [8, 7], [8, 5], [8, 4], [8, 3], [8, 2], [8, 1], [8, 0]];
	for (const [x, y] of first) a = (a << 1) | get(x, y);
	let b = 0;
	const second = [];
	for (let i = 0; i < 7; i++) second.push([8, size - 1 - i]);
	for (let i = 0; i < 8; i++) second.push([size - 8 + i, 8]);
	for (const [x, y] of second) b = (b << 1) | get(x, y);
	const word = (data) => ((data << 10) | gf2mod(data << 10, 0x537, 10)) ^ 0x5412;
	let best = null;
	for (let data = 0; data < 32; data++) {
		const d = Math.min(popcount(word(data) ^ a), popcount(word(data) ^ b));
		if (!best || d < best.d) best = { d, data };
	}
	if (best.d > 3) throw new Error('unreadable format information');
	const exact = word(best.data) === a && word(best.data) === b;
	return { level: best.data >> 3, mask: best.data & 7, exact };
}

function readVersion(size, get) {
	// The block above the bottom-left finder: bit i at (i / 3, size - 11 + i % 3).
	let v = 0;
	for (let i = 17; i >= 0; i--) v = (v << 1) | get(Math.floor(i / 3), size - 11 + (i % 3));
	return v;
}

function functionMap(version, size) {
	const f = new Uint8Array(size * size);
	const mark = (x, y) => {
		if (x >= 0 && y >= 0 && x < size && y < size) f[y * size + x] = 1;
	};
	for (let i = 0; i < size; i++) {
		mark(6, i);
		mark(i, 6);
	}
	for (let y = 0; y < 9; y++) for (let x = 0; x < 9; x++) mark(x, y);
	for (let y = 0; y < 9; y++) for (let x = size - 8; x < size; x++) mark(x, y);
	for (let y = size - 8; y < size; y++) for (let x = 0; x < 9; x++) mark(x, y);
	const c = ALIGN[version];
	for (const ax of c) {
		for (const ay of c) {
			if ((ax === 6 && ay === 6) || (ax === 6 && ay === c[c.length - 1]) || (ax === c[c.length - 1] && ay === 6)) continue;
			for (let dy = -2; dy <= 2; dy++) for (let dx = -2; dx <= 2; dx++) mark(ax + dx, ay + dy);
		}
	}
	if (version >= 7) {
		for (let i = 0; i < 6; i++) for (let j = 0; j < 3; j++) {
			mark(i, size - 11 + j);
			mark(size - 11 + j, i);
		}
	}
	return f;
}

const MASK = [
	(i, j) => (i + j) % 2 === 0,
	(i) => i % 2 === 0,
	(i, j) => j % 3 === 0,
	(i, j) => (i + j) % 3 === 0,
	(i, j) => (Math.floor(i / 2) + Math.floor(j / 3)) % 2 === 0,
	(i, j) => ((i * j) % 2) + ((i * j) % 3) === 0,
	(i, j) => (((i * j) % 2) + ((i * j) % 3)) % 2 === 0,
	(i, j) => (((i * j) % 3) + ((i + j) % 2)) % 2 === 0,
];

// GF(256) for the syndrome check.
const EXP = [];
const LOG = [];
for (let i = 0, x = 1; i < 255; i++) {
	EXP[i] = x;
	LOG[x] = i;
	x = (x << 1) ^ (x & 0x80 ? 0x11d : 0);
	x &= 0xff;
}
const mul = (a, b) => (a && b ? EXP[(LOG[a] + LOG[b]) % 255] : 0);

function syndromesZero(block, ecLength) {
	for (let i = 0; i < ecLength; i++) {
		let s = 0;
		for (const c of block) s = mul(s, EXP[i]) ^ c; // Horner at alpha^i
		if (s !== 0) return false;
	}
	return true;
}

/**
 * Decode a symbol given as its size and a module getter `get(x, y)` (1 dark). Returns
 * `{ text, bytes, version, mask }`.
 */
export function decodeQr(size, get) {
	if ((size - 17) % 4 !== 0) throw new Error('not a QR size');
	const version = (size - 17) / 4;
	if (!BLOCKS_M[version]) throw new Error(`version ${version} not supported here`);
	const { level, mask, exact } = readFormat(size, get);
	if (!exact) throw new Error('format information damaged');
	if (level !== 0) throw new Error('not level M');
	if (version >= 7) {
		const v = readVersion(size, get);
		if (v >> 12 !== version || gf2mod(v, 0x1f25, 12) !== 0) throw new Error('bad version information');
	}
	if (get(8, size - 8) !== 1) throw new Error('no dark module');
	// Timing patterns.
	for (let i = 8; i < size - 8; i++) {
		if (get(i, 6) !== (i % 2 === 0 ? 1 : 0) || get(6, i) !== (i % 2 === 0 ? 1 : 0)) throw new Error('bad timing pattern');
	}
	const fn = functionMap(version, size);
	const bits = [];
	let up = false;
	for (let right = size - 1; right > 0; right -= 2) {
		if (right === 6) right = 5; // the vertical timing pattern is skipped
		up = !up; // columns pairs alternate, starting upwards from the bottom right
		for (let k = 0; k < size; k++) {
			const y = up ? size - 1 - k : k;
			for (const x of [right, right - 1]) {
				if (fn[y * size + x]) continue;
				bits.push(get(x, y) ^ (MASK[mask](y, x) ? 1 : 0));
			}
		}
	}
	const [ecLength, groups] = BLOCKS_M[version];
	const lengths = groups.flatMap(([n, d]) => Array(n).fill(d));
	const total = lengths.reduce((a, b) => a + b, 0) + lengths.length * ecLength;
	const words = [];
	for (let i = 0; i + 8 <= bits.length && words.length < total; i += 8) {
		words.push(bits.slice(i, i + 8).reduce((a, b) => (a << 1) | b, 0));
	}
	if (words.length !== total) throw new Error('wrong number of codewords');
	const blocks = lengths.map(() => []);
	let at = 0;
	for (let i = 0; i < Math.max(...lengths); i++) lengths.forEach((n, b) => i < n && blocks[b].push(words[at++]));
	for (let i = 0; i < ecLength; i++) blocks.forEach((b) => b.push(words[at++]));
	blocks.forEach((b, i) => {
		if (!syndromesZero(b, ecLength)) throw new Error(`block ${i} fails its error correction check`);
	});
	const data = blocks.flatMap((b, i) => b.slice(0, lengths[i]));
	const stream = data.flatMap((w) => [7, 6, 5, 4, 3, 2, 1, 0].map((s) => (w >> s) & 1));
	let p = 0;
	const take = (n) => {
		let v = 0;
		for (let i = 0; i < n; i++) v = (v << 1) | stream[p++];
		return v;
	};
	if (take(4) !== 0b0100) throw new Error('not byte mode');
	const count = take(version < 10 ? 8 : 16);
	const bytes = [];
	for (let i = 0; i < count; i++) bytes.push(take(8));
	if (p > stream.length) throw new Error('count runs past the data');
	const rest = stream.length - p;
	if (rest >= 4 && take(4) !== 0) throw new Error('no terminator');
	return { text: new TextDecoder('utf-8', { fatal: true }).decode(new Uint8Array(bytes)), bytes, version, mask };
}
