// GENERATED from web/app/src/lib/qr/qr.js by tools/sync-qr.mjs: edit that file, then run
// `node tools/sync-qr.mjs`. Sets globalThis.AnywhereQR for the settings page.
(function () {
'use strict';
//
// QR codes for pairing links, made here: the project runs no third-party code in the
// browser. A small encoder after ISO/IEC 18004: byte mode, error correction level M
// (about 15% of the symbol can be damaged), versions 1 to 10 (up to 213 bytes, plenty for
// a pairing link). It is tested against the `qrcode` Rust crate's output for every mask,
// and by decoding what it makes (web/app/tests/qr.test.ts, plugin/t/js/qr.test.mjs).
//
// The LMS plugin serves a copy of this file as a classic script, made by
// tools/sync-qr.mjs (its tests check the copy is current). Keep it ASCII only.

/** Error correction for level M per version: [codewords per block, [blocks, data codewords]...]. */
const EC_M = [
	null,
	[10, [1, 16]],
	[16, [1, 28]],
	[26, [1, 44]],
	[18, [2, 32]],
	[24, [2, 43]],
	[16, [4, 27]],
	[18, [4, 31]],
	[22, [2, 38], [2, 39]],
	[22, [3, 36], [2, 37]],
	[26, [4, 43], [1, 44]],
];

/** Centres of the alignment patterns per version. */
const ALIGNMENT = [null, [], [6, 18], [6, 22], [6, 26], [6, 30], [6, 34], [6, 22, 38], [6, 24, 42], [6, 26, 46], [6, 28, 50]];

const MAX_VERSION = 10;

function dataCodewords(version) {
	let n = 0;
	for (const [blocks, data] of EC_M[version].slice(1)) n += blocks * data;
	return n;
}

/** Bytes a version holds in byte mode at level M. */
function capacity(version) {
	const countBits = version < 10 ? 8 : 16;
	return Math.floor((dataCodewords(version) * 8 - 4 - countBits) / 8);
}

/** UTF-8 bytes of a string (no TextEncoder needed). */
function utf8(text) {
	const out = [];
	for (const ch of String(text)) {
		let c = ch.codePointAt(0);
		if (c >= 0xd800 && c <= 0xdfff) c = 0xfffd; // a lone surrogate
		if (c < 0x80) out.push(c);
		else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 63));
		else if (c < 0x10000) out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
		else out.push(0xf0 | (c >> 18), 0x80 | ((c >> 12) & 63), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
	}
	return out;
}

// --- Reed-Solomon over GF(256), primitive polynomial x^8 + x^4 + x^3 + x^2 + 1 ----------

const EXP = new Array(512);
const LOG = new Array(256).fill(0);
{
	let x = 1;
	for (let i = 0; i < 255; i++) {
		EXP[i] = x;
		LOG[x] = i;
		x <<= 1;
		if (x & 0x100) x ^= 0x11d;
	}
	for (let i = 255; i < 512; i++) EXP[i] = EXP[i - 255];
}

function gfMul(a, b) {
	return a === 0 || b === 0 ? 0 : EXP[LOG[a] + LOG[b]];
}

/** The generator polynomial (x - a^0)(x - a^1)...(x - a^(n-1)), highest power first. */
function generator(n) {
	let g = [1];
	for (let i = 0; i < n; i++) {
		const next = new Array(g.length + 1).fill(0);
		for (let j = 0; j < g.length; j++) {
			next[j] ^= g[j];
			next[j + 1] ^= gfMul(g[j], EXP[i]);
		}
		g = next;
	}
	return g;
}

/** The n error correction codewords for a block of data codewords. */
function reedSolomon(data, n) {
	const g = generator(n);
	const rem = new Array(n).fill(0);
	for (const d of data) {
		const factor = d ^ rem.shift();
		rem.push(0);
		for (let i = 0; i < n; i++) rem[i] ^= gfMul(g[i + 1], factor);
	}
	return rem;
}

// --- the codewords ---------------------------------------------------------------------

/** Data and error correction codewords, interleaved, for `bytes` in `version`. */
function codewords(bytes, version) {
	const total = dataCodewords(version);
	const bits = [];
	const push = (value, length) => {
		for (let i = length - 1; i >= 0; i--) bits.push((value >>> i) & 1);
	};
	push(0b0100, 4); // byte mode
	push(bytes.length, version < 10 ? 8 : 16);
	for (const b of bytes) push(b, 8);
	push(0, Math.min(4, total * 8 - bits.length)); // terminator
	while (bits.length % 8) bits.push(0);
	const data = [];
	for (let i = 0; i < bits.length; i += 8) {
		let b = 0;
		for (let j = 0; j < 8; j++) b = (b << 1) | bits[i + j];
		data.push(b);
	}
	for (let pad = 0xec; data.length < total; pad ^= 0xec ^ 0x11) data.push(pad);

	const [ecLength, ...groups] = EC_M[version];
	const blocks = [];
	let at = 0;
	for (const [count, length] of groups) {
		for (let i = 0; i < count; i++) {
			const block = data.slice(at, at + length);
			at += length;
			blocks.push({ data: block, ec: reedSolomon(block, ecLength) });
		}
	}
	const out = [];
	const longest = Math.max(...blocks.map((b) => b.data.length));
	for (let i = 0; i < longest; i++) for (const b of blocks) if (i < b.data.length) out.push(b.data[i]);
	for (let i = 0; i < ecLength; i++) for (const b of blocks) out.push(b.ec[i]);
	return out;
}

// --- the symbol --------------------------------------------------------------------------

const MASKS = [
	(x, y) => (x + y) % 2 === 0,
	(x, y) => y % 2 === 0,
	(x, y) => x % 3 === 0,
	(x, y) => (x + y) % 3 === 0,
	(x, y) => (Math.floor(y / 2) + Math.floor(x / 3)) % 2 === 0,
	(x, y) => ((x * y) % 2) + ((x * y) % 3) === 0,
	(x, y) => (((x * y) % 2) + ((x * y) % 3)) % 2 === 0,
	(x, y) => (((x + y) % 2) + ((x * y) % 3)) % 2 === 0,
];

/** The 15 format bits for level M and a mask (BCH code, then the fixed XOR mask). */
function formatBits(mask) {
	const data = (0b00 << 3) | mask; // level M is 00
	let rem = data;
	for (let i = 0; i < 10; i++) rem = (rem << 1) ^ ((rem >>> 9) * 0x537);
	return ((data << 10) | rem) ^ 0x5412;
}

/** The 18 version bits (versions 7 and up). */
function versionBits(version) {
	let rem = version;
	for (let i = 0; i < 12; i++) rem = (rem << 1) ^ ((rem >>> 11) * 0x1f25);
	return (version << 12) | rem;
}

function makeGrid(version) {
	const size = version * 4 + 17;
	const dark = new Uint8Array(size * size);
	const fixed = new Uint8Array(size * size);
	const set = (x, y, on) => {
		dark[y * size + x] = on ? 1 : 0;
		fixed[y * size + x] = 1;
	};
	for (let i = 0; i < size; i++) {
		set(6, i, i % 2 === 0);
		set(i, 6, i % 2 === 0);
	}
	for (const [cx, cy] of [[3, 3], [size - 4, 3], [3, size - 4]]) {
		for (let dy = -4; dy <= 4; dy++) {
			for (let dx = -4; dx <= 4; dx++) {
				const x = cx + dx, y = cy + dy;
				if (x < 0 || y < 0 || x >= size || y >= size) continue;
				const d = Math.max(Math.abs(dx), Math.abs(dy));
				set(x, y, d !== 2 && d !== 4);
			}
		}
	}
	const centres = ALIGNMENT[version];
	const last = centres.length - 1;
	for (let i = 0; i <= last; i++) {
		for (let j = 0; j <= last; j++) {
			if ((i === 0 && j === 0) || (i === 0 && j === last) || (i === last && j === 0)) continue;
			for (let dy = -2; dy <= 2; dy++) {
				for (let dx = -2; dx <= 2; dx++) set(centres[i] + dx, centres[j] + dy, Math.max(Math.abs(dx), Math.abs(dy)) !== 1);
			}
		}
	}
	if (version >= 7) {
		const bits = versionBits(version);
		for (let i = 0; i < 18; i++) {
			const on = ((bits >>> i) & 1) === 1;
			const a = size - 11 + (i % 3), b = Math.floor(i / 3);
			set(a, b, on);
			set(b, a, on);
		}
	}
	return { size, dark, fixed, set };
}

function drawFormat(grid, mask) {
	const { size, set } = grid;
	const bits = formatBits(mask);
	const bit = (i) => ((bits >>> i) & 1) === 1;
	for (let i = 0; i <= 5; i++) set(8, i, bit(i));
	set(8, 7, bit(6));
	set(8, 8, bit(7));
	set(7, 8, bit(8));
	for (let i = 9; i < 15; i++) set(14 - i, 8, bit(i));
	for (let i = 0; i < 8; i++) set(size - 1 - i, 8, bit(i));
	for (let i = 8; i < 15; i++) set(8, size - 15 + i, bit(i));
	set(8, size - 8, true); // the dark module
}

function placeData(grid, words) {
	const { size, dark, fixed } = grid;
	let i = 0;
	const n = words.length * 8;
	for (let right = size - 1; right >= 1; right -= 2) {
		if (right === 6) right = 5;
		const upward = ((right + 1) & 2) === 0;
		for (let v = 0; v < size; v++) {
			const y = upward ? size - 1 - v : v;
			for (let j = 0; j < 2; j++) {
				const x = right - j;
				if (fixed[y * size + x]) continue;
				// Remainder modules past the last codeword stay light (before masking).
				if (i < n) dark[y * size + x] = (words[i >> 3] >>> (7 - (i & 7))) & 1;
				i++;
			}
		}
	}
}

function applyMask(grid, mask) {
	const { size, dark, fixed } = grid;
	const fn = MASKS[mask];
	for (let y = 0; y < size; y++) {
		for (let x = 0; x < size; x++) {
			if (!fixed[y * size + x] && fn(x, y)) dark[y * size + x] ^= 1;
		}
	}
}

/** The standard penalty score of a finished symbol (lower reads more reliably). */
function penalty(size, dark) {
	let score = 0;
	const at = (x, y) => dark[y * size + x];
	for (let pass = 0; pass < 2; pass++) {
		const get = pass === 0 ? at : (x, y) => at(y, x);
		for (let y = 0; y < size; y++) {
			let run = 1;
			for (let x = 1; x <= size; x++) {
				if (x < size && get(x, y) === get(x - 1, y)) {
					run++;
				} else {
					if (run >= 5) score += run - 2;
					run = 1;
				}
			}
			// Finder-like runs: dark-light-dark x3-light-dark with four light modules on a side.
			for (let x = 0; x + 11 <= size; x++) {
				let a = true, b = true;
				for (let k = 0; k < 11; k++) {
					const m = get(x + k, y);
					if (m !== [1, 0, 1, 1, 1, 0, 1, 0, 0, 0, 0][k]) a = false;
					if (m !== [0, 0, 0, 0, 1, 0, 1, 1, 1, 0, 1][k]) b = false;
				}
				if (a) score += 40;
				if (b) score += 40;
			}
		}
	}
	for (let y = 0; y + 1 < size; y++) {
		for (let x = 0; x + 1 < size; x++) {
			const c = at(x, y);
			if (c === at(x + 1, y) && c === at(x, y + 1) && c === at(x + 1, y + 1)) score += 3;
		}
	}
	let total = 0;
	for (const m of dark) total += m;
	const k = Math.ceil(Math.abs(total * 20 - size * size * 10) / (size * size)) - 1;
	score += Math.max(0, k) * 10;
	return score;
}

/**
 * Encode text (as UTF-8) in the smallest version that holds it. `mask` forces a mask
 * pattern (0-7); otherwise the one with the lowest penalty is used. Returns the symbol
 * as rows of modules, row-major, 1 for dark. Throws a RangeError for text too long.
 */
function encodeQr(text, options = {}) {
	const bytes = utf8(text);
	let version = 1;
	while (version <= MAX_VERSION && bytes.length > capacity(version)) version++;
	if (version > MAX_VERSION) throw new RangeError('too long for a QR code here');
	const words = codewords(bytes, version);
	const build = (mask) => {
		const grid = makeGrid(version);
		drawFormat(grid, mask); // reserves the format areas before the data goes in
		placeData(grid, words);
		applyMask(grid, mask);
		drawFormat(grid, mask);
		return grid;
	};
	let best = null;
	if (Number.isInteger(options.mask) && options.mask >= 0 && options.mask < 8) {
		best = { mask: options.mask, grid: build(options.mask) };
	} else {
		let bestScore = Infinity;
		for (let mask = 0; mask < 8; mask++) {
			const grid = build(mask);
			const score = penalty(grid.size, grid.dark);
			if (score < bestScore) {
				bestScore = score;
				best = { mask, grid };
			}
		}
	}
	return { version, size: best.grid.size, mask: best.mask, modules: best.grid.dark };
}

/**
 * An SVG path drawing the dark modules (one rectangle per horizontal run), offset by a
 * quiet zone of `quiet` modules. The symbol is `size + 2 * quiet` units square.
 */
function qrPath(qr, quiet = 4) {
	const { size, modules } = qr;
	const parts = [];
	for (let y = 0; y < size; y++) {
		let x = 0;
		while (x < size) {
			if (!modules[y * size + x]) {
				x++;
				continue;
			}
			let run = 1;
			while (x + run < size && modules[y * size + x + run]) run++;
			parts.push(`M${x + quiet} ${y + quiet}h${run}v1h-${run}z`);
			x += run;
		}
	}
	return parts.join('');
}

// --- pairing links -------------------------------------------------------------------------

/** The scheme of pairing links (docs/PROTOCOL.md, "Pairing links"). */
const PAIRING_SCHEME = 'lyrion-anywhere';

/** A pairing code as the gateway reads it (spaces and dashes dropped, upper case), or null. */
function normalisePairingCode(code) {
	if (typeof code !== 'string') return null;
	const c = code.replace(/[\s-]/g, '').toUpperCase();
	return /^[A-Z0-9]{4,32}$/.test(c) ? c : null;
}

function isLoopback(host) {
	return host === 'localhost' || host === '[::1]' || /^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(host);
}

/**
 * The server part of a pairing link: the origin of an `https://` address with nothing
 * after the host (plain `http://` only for loopback, as the apps accept), or null. The
 * same rules as the apps use, so a link made here is never one they refuse.
 */
function pairingServer(address) {
	if (typeof address !== 'string') return null;
	const s = address.trim().replace(/\/+$/, '');
	if (!s || s.length > 2048 || /[\s\\\u0000-\u001f\u007f]/.test(s)) return null;
	let u;
	try {
		u = new URL(s);
	} catch {
		return null;
	}
	if (u.protocol !== 'https:' && !(u.protocol === 'http:' && isLoopback(u.hostname))) return null;
	if (!u.hostname || u.username || u.password || u.search || u.hash || u.pathname !== '/') return null;
	// Nothing the URL parser quietly dropped ("https://host/?" or "https://host#").
	if (/[?#@]/.test(s.slice(s.indexOf('//') + 2))) return null;
	return u.origin;
}

/** `lyrion-anywhere://pair?server=...&code=...`, or null if either part isn't valid. */
function pairingLink(server, code) {
	const origin = pairingServer(server);
	const c = normalisePairingCode(code);
	if (!origin || !c) return null;
	return `${PAIRING_SCHEME}://pair?server=${encodeURIComponent(origin)}&code=${c}`;
}

/** A pairing code grouped for reading aloud or typing: "ABCDE-FGHJK". */
function groupPairingCode(code) {
	const c = normalisePairingCode(code) ?? '';
	return c.length === 10 ? `${c.slice(0, 5)}-${c.slice(5)}` : c;
}

globalThis.AnywhereQR = Object.freeze({ MAX_VERSION, capacity, utf8, reedSolomon, formatBits, versionBits, penalty, encodeQr, qrPath, PAIRING_SCHEME, normalisePairingCode, pairingServer, pairingLink, groupPairingCode });
})();
