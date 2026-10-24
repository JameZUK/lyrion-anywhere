// The in-house QR encoder (src/lib/qr/qr.js): module for module against the qrcode crate's
// symbols for every mask, decoded back by an independent decoder, and robust on any input.
import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'vitest';
import {
	MAX_VERSION,
	capacity,
	encodeQr,
	groupPairingCode,
	isLoopback,
	isPrivateLiteral,
	normalisePairingCode,
	pairingLink,
	pairingServer,
	penalty,
	qrPath,
	reedSolomon,
	utf8,
	type QrSymbol,
} from '../src/lib/qr/qr.js';
import { decodeQr } from './helpers/qr-decode.mjs';

interface Vector {
	text: string;
	version: number;
	masks: string[][];
}
/** The trusted-networks vectors shared with core::privnet and the plugin. */
const privnet = JSON.parse(readFileSync(`${__dirname}/fixtures/private-networks.json`, 'utf8')) as { addresses: { private: string[]; not_private: string[] } };
const { vectors } = JSON.parse(readFileSync(`${__dirname}/fixtures/qr-vectors.json`, 'utf8')) as { vectors: Vector[] };

/** Rows as hex, the fixture's form. */
function rowsHex(qr: QrSymbol): string[] {
	const out: string[] = [];
	for (let y = 0; y < qr.size; y++) {
		const bits = Array.from(qr.modules.slice(y * qr.size, (y + 1) * qr.size));
		while (bits.length % 4) bits.push(0);
		let row = '';
		for (let i = 0; i < bits.length; i += 4) row += bits.slice(i, i + 4).reduce((a, b) => (a << 1) | b, 0).toString(16);
		out.push(row);
	}
	return out;
}

const decode = (qr: QrSymbol) => decodeQr(qr.size, (x, y) => qr.modules[y * qr.size + x] ?? 0);

/** A deterministic generator of awkward strings. */
function* strings(count: number, seed = 0x2545f491) {
	const alphabet = [...'abcXYZ019:/?&=%#@ -_.~', 'é', '☕', '\u{1f3b5}', '\u0000', '‮', '\ud800'];
	let s = seed;
	const next = () => {
		s ^= s << 13;
		s ^= s >>> 17;
		s ^= s << 5;
		return s >>> 0;
	};
	for (let i = 0; i < count; i++) {
		const len = next() % 220;
		let out = '';
		for (let j = 0; j < len; j++) out += alphabet[next() % alphabet.length];
		yield out;
	}
}

describe('QR encoder', () => {
	test('the fixture covers versions 1 to 10', () => {
		expect(vectors.length).toBeGreaterThanOrEqual(8);
		expect(new Set(vectors.map((v) => v.version))).toEqual(new Set([1, 2, 3, 5, 7, 8, 10]));
	});

	test('matches the qrcode crate module for module, for every mask', () => {
		for (const v of vectors) {
			for (let mask = 0; mask < 8; mask++) {
				const qr = encodeQr(v.text, { mask });
				expect(qr.version, v.text).toBe(v.version);
				expect(qr.mask).toBe(mask);
				expect(rowsHex(qr), `${v.text.slice(0, 30)} mask ${mask}`).toEqual(v.masks[mask]);
			}
		}
	});

	test('the decoder reads the crate symbols (so it checks the encoder independently)', () => {
		for (const v of vectors) {
			v.masks.forEach((rows, mask) => {
				const size = rows.length;
				const get = (x: number, y: number) => (parseInt(rows[y]![x >> 2]!, 16) >> (3 - (x & 3))) & 1;
				const d = decodeQr(size, get);
				expect(d.text).toBe(v.text);
				expect(d.mask).toBe(mask);
			});
		}
	});

	test('symbols it makes decode back to the text, with the mask of lowest penalty', () => {
		for (const text of ['A', 'lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com&code=ABCDEFGHJK', 'x'.repeat(213)]) {
			const qr = encodeQr(text);
			expect(decode(qr).text).toBe(text);
			const scores = [0, 1, 2, 3, 4, 5, 6, 7].map((m) => {
				const s = encodeQr(text, { mask: m });
				return penalty(s.size, s.modules);
			});
			expect(scores[qr.mask]).toBe(Math.min(...scores));
			expect(scores.indexOf(Math.min(...scores))).toBe(qr.mask);
		}
	});

	test('arbitrary text round-trips, and never throws except for being too long', () => {
		let encoded = 0;
		for (const text of strings(150)) {
			const bytes = utf8(text);
			if (bytes.length > capacity(MAX_VERSION)) {
				expect(() => encodeQr(text)).toThrow(RangeError);
				continue;
			}
			const qr = encodeQr(text);
			expect(qr.size).toBe(qr.version * 4 + 17);
			expect(qr.modules.length).toBe(qr.size * qr.size);
			expect(decode(qr).bytes).toEqual(bytes);
			encoded++;
		}
		expect(encoded).toBeGreaterThan(50);
	});

	test('picks the smallest version, up to 213 bytes', () => {
		expect([1, 2, 3, 4, 5, 6, 7, 8, 9, 10].map(capacity)).toEqual([14, 26, 42, 62, 84, 106, 122, 152, 180, 213]);
		for (let v = 1; v <= MAX_VERSION; v++) {
			expect(encodeQr('a'.repeat(capacity(v))).version).toBe(v);
			if (v < MAX_VERSION) expect(encodeQr('a'.repeat(capacity(v) + 1)).version).toBe(v + 1);
		}
		expect(() => encodeQr('a'.repeat(214))).toThrow(RangeError);
		expect(encodeQr('').version).toBe(1);
		expect(decode(encodeQr('')).text).toBe('');
	});

	test('damage is noticed by the decoder (the check itself works)', () => {
		const qr = encodeQr('lyrion-anywhere://pair?code=ABCDE', { mask: 2 });
		const broken = { ...qr, modules: qr.modules.slice() };
		const i = (qr.size - 1) * qr.size + qr.size - 1; // the first data module
		broken.modules[i] = broken.modules[i] ? 0 : 1;
		expect(() => decode(broken)).toThrow(/error correction/);
	});

	test('Reed-Solomon matches the worked example in ISO/IEC 18004 annex I', () => {
		// "01234567", version 1-M: 16 data codewords and their 10 error correction codewords.
		const data = [0x10, 0x20, 0x0c, 0x56, 0x61, 0x80, 0xec, 0x11, 0xec, 0x11, 0xec, 0x11, 0xec, 0x11, 0xec, 0x11];
		expect(reedSolomon(data, 10)).toEqual([0xa5, 0x24, 0xd4, 0xc1, 0xed, 0x36, 0xc7, 0x87, 0x2c, 0x55]);
	});

	test('the SVG path draws exactly the dark modules, inside the quiet zone', () => {
		const qr = encodeQr('https://music.example.com/');
		const path = qrPath(qr, 4);
		expect(path).toMatch(/^(M\d+ \d+h\d+v1h-\d+z)+$/);
		let dark = 0;
		for (const m of path.matchAll(/M(\d+) (\d+)h(\d+)/g)) {
			const [x, y, run] = [Number(m[1]), Number(m[2]), Number(m[3])];
			expect(x).toBeGreaterThanOrEqual(4);
			expect(y).toBeGreaterThanOrEqual(4);
			expect(x + run).toBeLessThanOrEqual(qr.size + 4);
			for (let k = 0; k < run; k++) expect(qr.modules[(y - 4) * qr.size + x - 4 + k]).toBe(1);
			dark += run;
		}
		expect(dark).toBe(qr.modules.reduce((a, b) => a + b, 0));
	});
});

describe('pairing links', () => {
	test('a link holds the server origin and the normalised code', () => {
		expect(pairingLink('https://music.example.com', 'abcde-fghjk')).toBe('lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com&code=ABCDEFGHJK');
		expect(pairingLink('https://music.example.com/', 'ABCDEFGHJK')).toBe('lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com&code=ABCDEFGHJK');
		expect(pairingLink('https://music.example.com:8443', 'ABCD')).toBe('lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com%3A8443&code=ABCD');
		expect(pairingLink('https://music.example.com:443', 'ABCD')).toBe('lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com&code=ABCD');
		// Local development only.
		expect(pairingServer('http://localhost:5173')).toBe('http://localhost:5173');
		expect(pairingServer('http://127.0.0.1:9443')).toBe('http://127.0.0.1:9443');
		expect(pairingServer('http://[::1]:9443')).toBe('http://[::1]:9443');
	});

	test('refuses servers the apps would refuse', () => {
		for (const bad of [
			'',
			'music.example.com',
			'http://music.example.com',
			'http://10.0.0.10.example.com',
			'http://8.8.8.8',
			'ftp://music.example.com',
			'ftp://10.0.0.10',
			'javascript:alert(1)',
			'https://user@music.example.com',
			'https://user:pw@music.example.com',
			'https://music.example.com/lms',
			'https://music.example.com/?x=1',
			'https://music.example.com/?',
			'https://music.example.com#x',
			'https://music.example.com#',
			'https://music.example.com\\@evil.example',
			'https://music.example .com',
			'https://music.exam\nple.com',
			'https://',
			'https://' + 'a'.repeat(2050) + '.example.com',
			null,
			42,
			{ toString: () => 'https://music.example.com' },
		]) {
			expect(pairingServer(bad), String(bad).slice(0, 40)).toBeNull();
			expect(pairingLink(bad, 'ABCDEFGHJK')).toBeNull();
		}
	});

	test('plain http:// to a literal private address (a trusted network) is accepted, from the shared vectors', () => {
		const bracket = (a: string) => (a.includes(':') ? `[${a}]` : a);
		for (const a of privnet.addresses.private) {
			const host = bracket(a);
			expect(isPrivateLiteral(host), host).toBe(true);
			expect(pairingServer(`http://${host}`), host).toBe(`http://${host}`);
			expect(pairingServer(`http://${host}:9443/`), host).toBe(`http://${host}:9443`);
			expect(pairingLink(`http://${host}:9443`, 'ABCD'), host).toBe(`lyrion-anywhere://pair?server=${encodeURIComponent(`http://${host}:9443`)}&code=ABCD`);
		}
		for (const a of privnet.addresses.not_private) {
			const host = bracket(a);
			expect(isPrivateLiteral(host), host).toBe(false);
			const loopback = host === '127.0.0.1' || host === '[::1]';
			for (const url of [`http://${host}`, `http://${host}:9443`]) {
				if (loopback) expect(pairingServer(url), url).toBe(url);
				else expect(pairingServer(url), url).toBeNull();
			}
			// https:// is a different matter: any address the certificate covers.
		}
	});

	test('isPrivateLiteral takes literals only: never a name, loopback, a mapped or malformed address', () => {
		for (const bad of [
			'',
			'localhost',
			'music.example.com',
			'10.0.0.1.example.com',
			'example.10.0.0.1',
			'10.0.0.1.',
			'10.0.0',
			'10.0.0.0.1',
			'010.0.0.1',
			'10.0.0.256',
			'10.0.0.-1',
			' 10.0.0.1',
			'10.0.0.1 ',
			'0x0a.0.0.1',
			'127.0.0.1',
			'fd00::1', // IPv6 without brackets: not how a URL's hostname reads
			'[fd00::1',
			'fd00::1]',
			'[fd00::1%eth0]',
			'[fe80::1%25eth0]',
			'[fd00:::1]',
			'[fd00::1::2]',
			'[fd00:1:2:3:4:5:6:7:8]',
			'[fd00:1:2:3:4:5:6]',
			'[fd00::12345]',
			'[fd00::g]',
			'[::ffff:10.0.0.1]',
			'[::ffff:a00:1]',
			'[::]',
			'[::1]',
			'[fec0::1]',
			'[2001:db8::1]',
			'[' + 'f'.repeat(50) + ']',
			null,
			undefined,
			10,
			{ toString: () => '10.0.0.1' },
		]) {
			expect(isPrivateLiteral(bad), String(bad)).toBe(false);
		}
		for (const good of ['10.8.0.1', '100.64.0.1', '[fd12:3456:789a::1]', '[FD12:3456:789A::1]', '[fe80::]', '[fd00:0:0:0:0:0:0:1]', '[fc00::]']) {
			expect(isPrivateLiteral(good), good).toBe(true);
		}
		// Arbitrary input never throws, and nothing outside the ranges gets through.
		let seed = 7;
		const rnd = () => ((seed = (seed * 1103515245 + 12345) >>> 0) / 2 ** 32);
		const alphabet = '0123456789abcdefABCDEF:.[]%/-xg ';
		for (let i = 0; i < 20_000; i++) {
			const s = Array.from({ length: Math.floor(rnd() * 24) }, () => alphabet[Math.floor(rnd() * alphabet.length)]).join('');
			const ok = isPrivateLiteral(s);
			if (ok) expect(/^(10|172|192|169|100)\.|^\[f[cdeE]/i.test(s), s).toBe(true);
			expect(() => pairingServer(`http://${s}`)).not.toThrow();
		}
		expect(isLoopback('localhost') && isLoopback('127.0.0.1') && isLoopback('127.1.2.3') && isLoopback('[::1]')).toBe(true);
		for (const h of ['10.0.0.1', 'localhost.example.com', '::1', '[::2]', '128.0.0.1']) expect(isLoopback(h), h).toBe(false);
	});

	test('the URL parser normalises before the rule applies (no surprises from odd spellings)', () => {
		// WHATWG URLs read 10.1 and 0xa.0.0.1 as 10.0.0.1, and 010.0.0.1 (octal) as 8.0.0.1:
		// the rule sees the normalised address, the same one the browser would connect to.
		expect(pairingServer('http://10.1:9443')).toBe('http://10.0.0.1:9443');
		expect(pairingServer('http://0xa.0.0.1')).toBe('http://10.0.0.1');
		expect(pairingServer('http://010.0.0.1')).toBeNull();
		expect(pairingServer('http://[FD00::0001]')).toBe('http://[fd00::1]');
		expect(pairingServer('http://[::ffff:10.0.0.1]')).toBeNull();
		expect(pairingServer('http://10.0.0.1@music.example.com')).toBeNull();
		expect(pairingServer('http://music.example.com@10.0.0.1')).toBeNull();
		expect(pairingServer('http://10.0.0.1/lms')).toBeNull();
	});

	test('refuses codes the gateway could never have made', () => {
		for (const bad of ['', 'abc', 'ABCD!EFGH', 'A'.repeat(33), 'ABCD&server=x', 'ABCD#', 'АBCDE', null, 12345]) {
			expect(normalisePairingCode(bad)).toBeNull();
			expect(pairingLink('https://music.example.com', bad)).toBeNull();
		}
		expect(normalisePairingCode(' ab cd-ef ')).toBe('ABCDEF');
		expect(groupPairingCode('abcdefghjk')).toBe('ABCDE-FGHJK');
	});

	test('a pairing link fits a small symbol and decodes intact', () => {
		const link = pairingLink('https://a-rather-long-host-name.music.example.com:8443', 'ABCDEFGHJK')!;
		const qr = encodeQr(link);
		expect(qr.version).toBeLessThanOrEqual(7);
		expect(decode(qr).text).toBe(link);
	});
});
