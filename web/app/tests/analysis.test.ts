// Track analysis for the waveform seek bar: the binary decoder (untrusted bytes: refusals and
// a robustness run), the drawing maths, the session cache, and the component's fallback.
import { render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { BINARY_TYPE, MAX_BUCKETS, MAX_BYTES, decodeAnalysis, type TrackAnalysis } from '$lib/analysis/format';
import { BAR_PITCH, MAX_BARS, MIN_BARS, barCount, bars, barsPath, fraction, level } from '$lib/analysis/waveform';
import { AnalysisClient, analysisUrl, retryAfter, type Fetcher } from '$lib/analysis/client';
import SeekBar from '$lib/components/SeekBar.svelte';

// ---------------------------------------------------------------------------
// An encoder (test only), mirroring core::analysis::encode.
// ---------------------------------------------------------------------------

interface Enc {
	version?: number;
	rate?: number;
	channels?: number;
	frames?: number;
	floats?: number[]; // 6 optional values (NaN = none)
	silences?: [number, number];
	fade?: number; // fade flag byte
	peak?: number[];
	rms?: number[];
	n?: number; // bucket count written (defaults to peak.length)
}

function encode(e: Enc = {}): Uint8Array {
	const peak = e.peak ?? [0, 64, 128, 255];
	const rms = e.rms ?? peak.map((p) => p >> 1);
	const out: number[] = [];
	const dv = new DataView(new ArrayBuffer(8));
	const u8 = (v: number) => out.push(v & 255);
	const u16 = (v: number) => {
		dv.setUint16(0, v, true);
		out.push(dv.getUint8(0), dv.getUint8(1));
	};
	const u32 = (v: number) => {
		dv.setUint32(0, v, true);
		for (let i = 0; i < 4; i++) out.push(dv.getUint8(i));
	};
	const u64 = (v: number) => {
		u32(v % 2 ** 32);
		u32(Math.floor(v / 2 ** 32));
	};
	const f32 = (v: number) => {
		dv.setFloat32(0, v, true);
		for (let i = 0; i < 4; i++) out.push(dv.getUint8(i));
	};
	for (const c of 'LAAN') out.push(c.charCodeAt(0));
	u16(e.version ?? 1);
	u32(e.rate ?? 44_100);
	u16(e.channels ?? 2);
	u64(e.frames ?? 44_100 * 180);
	for (const v of e.floats ?? [-14.2, 6.5, -9.1, -11, -0.3, -0.5]) f32(v);
	f32(e.silences?.[0] ?? 0.25);
	f32(e.silences?.[1] ?? 1.5);
	for (let k = 0; k < 2; k++) {
		f32(k ? -3.2 : 2.1);
		f32(-40);
		f32(Number.NaN);
		f32(k ? 4.5 : 0);
		u8(e.fade ?? k);
	}
	u64(0);
	u16(e.n ?? peak.length);
	for (const p of peak) u8(p);
	for (const r of rms) u8(r);
	return new Uint8Array(out);
}

describe('decodeAnalysis', () => {
	it('reads every field', () => {
		const a = decodeAnalysis(encode())!;
		expect(a).not.toBeNull();
		expect(a.sampleRate).toBe(44_100);
		expect(a.channels).toBe(2);
		expect(a.frames).toBe(44_100 * 180);
		expect(a.durationS).toBe(180);
		expect(a.integratedLufs).toBe(-14.2);
		expect(a.loudnessRangeLu).toBe(6.5);
		expect(a.truePeakDbtp).toBe(-0.3);
		expect(a.samplePeakDbfs).toBe(-0.5);
		expect(a.leadingSilenceS).toBe(0.25);
		expect(a.trailingSilenceS).toBe(1.5);
		expect(a.fadeIn.fade).toBe(false);
		expect(a.fadeOut).toEqual({ slopeDbPerS: expect.closeTo(-3.2, 5), startLufs: -40, endLufs: null, lengthS: 4.5, fade: true });
		expect([...a.peak]).toEqual([0, 64, 128, 255]);
		expect([...a.rms]).toEqual([0, 32, 64, 127]);
	});

	it('takes an ArrayBuffer or a view at an offset', () => {
		const b = encode();
		const padded = new Uint8Array(b.length + 7);
		padded.set(b, 7);
		expect(decodeAnalysis(padded.subarray(7))).not.toBeNull();
		expect(decodeAnalysis(b.slice().buffer)).not.toBeNull();
	});

	it('reads NaN as "none" and a silent track as no loudness', () => {
		const a = decodeAnalysis(encode({ floats: Array(6).fill(Number.NaN), peak: [] }))!;
		expect(a.integratedLufs).toBeNull();
		expect(a.truePeakDbtp).toBeNull();
		expect(a.peak.length).toBe(0);
	});

	it('refuses malformed, oversized and foreign input', () => {
		const good = encode();
		const bad: Uint8Array[] = [
			new Uint8Array(0),
			new Uint8Array([0x4c, 0x41, 0x41]),
			encode({ version: 2 }),
			encode({ fade: 2 }),
			encode({ floats: [Infinity, 0, 0, 0, 0, 0] }),
			encode({ floats: [2e6, 0, 0, 0, 0, 0] }),
			encode({ silences: [Number.NaN, 0] }),
			encode({ frames: 2 ** 60 }),
			encode({ n: 5 }), // claims more buckets than are there
			encode({ n: MAX_BUCKETS + 1, peak: [] }),
			Uint8Array.from([...good, 0]), // trailing byte
			good.subarray(0, good.length - 1),
			new Uint8Array(MAX_BYTES + 1),
		];
		const wrongMagic = good.slice();
		wrongMagic[0] = 0x4d;
		bad.push(wrongMagic);
		for (const b of bad) expect(decodeAnalysis(b)).toBeNull();
	});

	it('never throws on hostile bytes (robustness)', () => {
		const base = encode({ peak: Array.from({ length: 200 }, (_, i) => i % 256) });
		let seed = 0x9e3779b9;
		const rnd = () => {
			seed ^= seed << 13;
			seed ^= seed >>> 17;
			seed ^= seed << 5;
			return seed >>> 0;
		};
		for (let i = 0; i < 20_000; i++) {
			let b = base.slice();
			switch (rnd() % 4) {
				case 0:
					b = b.subarray(0, rnd() % b.length);
					break;
				case 1:
					for (let k = 0; k < 1 + (rnd() % 8); k++) {
						const at = rnd() % b.length;
						b[at] = (b[at] ?? 0) ^ (1 << (rnd() % 8));
					}
					break;
				case 2:
					b = Uint8Array.from({ length: rnd() % 600 }, () => rnd() & 255);
					break;
				default: {
					// Rewrite the bucket count to anything.
					const at = b.length - 400 - 2;
					b[at] = rnd() & 255;
					b[at + 1] = rnd() & 255;
				}
			}
			let a: TrackAnalysis | null = null;
			expect(() => (a = decodeAnalysis(b))).not.toThrow();
			if (a) {
				const ok = a as TrackAnalysis;
				expect(ok.peak.length).toBe(ok.rms.length);
				expect(ok.peak.length).toBeLessThanOrEqual(MAX_BUCKETS);
				expect(Number.isFinite(ok.durationS)).toBe(true);
			}
		}
	});
});

// ---------------------------------------------------------------------------
// Drawing maths
// ---------------------------------------------------------------------------

describe('waveform maths', () => {
	it('fits bars to the width', () => {
		expect(barCount(300)).toBe(100);
		expect(barCount(1)).toBe(MIN_BARS);
		expect(barCount(100_000)).toBe(MAX_BARS);
		for (const w of [Number.NaN, -5, 0, Infinity]) expect(barCount(w)).toBe(MIN_BARS);
		expect(barCount(300, 0)).toBe(MIN_BARS);
		expect(BAR_PITCH).toBeGreaterThan(0);
	});

	it('keeps the loudest bucket of each bar', () => {
		const v = new Uint8Array(1000);
		v[537] = 255; // one short peak
		const b = bars(v, 10);
		expect(b).toHaveLength(10);
		expect(b[5]).toBe(1);
		expect(b.filter((x) => x > 0)).toHaveLength(1);
		// More bars than buckets: each bucket spans several bars.
		const few = bars(Uint8Array.from([0, 255]), 4);
		expect(few).toEqual([0, 0, 1, 1]);
		expect(bars(new Uint8Array(0), 10)).toEqual([]);
		expect(bars(v, 0)).toEqual([]);
		expect(bars(v, Number.NaN)).toEqual([]);
	});

	it('maps levels monotonically into 0–1 and tolerates junk', () => {
		expect(level(0)).toBe(0);
		expect(level(255)).toBe(1);
		expect(level(64)).toBeGreaterThan(64 / 255); // quiet parts lifted
		expect(level(500)).toBe(1);
		expect(level(-3)).toBe(0);
		expect(level(Number.NaN)).toBe(0);
		let last = -1;
		for (let v = 0; v <= 255; v++) {
			expect(level(v)).toBeGreaterThanOrEqual(last);
			last = level(v);
		}
	});

	it('draws centred bars inside the box', () => {
		const d = barsPath([0, 0.5, 1], 30, 40, 1, 2);
		const rects = d.split('M').filter(Boolean);
		expect(rects).toHaveLength(3);
		// Silence is a 2 px line in the middle; full scale fills the height.
		expect(rects[0]).toBe('0 19h9v2h-9z');
		expect(rects[1]).toBe('10 10h9v20h-9z');
		expect(rects[2]).toBe('20 0h9v40h-9z');
		expect(barsPath([], 30, 40)).toBe('');
		expect(barsPath([1], 0, 40)).toBe('');
		expect(barsPath([2, -1, Number.NaN], 30, 40)).not.toMatch(/NaN|Infinity|-\d+(\.\d+)? \d/);
	});

	it('gives the played fraction', () => {
		expect(fraction(30, 120)).toBe(0.25);
		expect(fraction(200, 120)).toBe(1);
		expect(fraction(-5, 120)).toBe(0);
		expect(fraction(10, 0)).toBe(0);
		expect(fraction(Number.NaN, 100)).toBe(0);
	});
});

// ---------------------------------------------------------------------------
// The session cache
// ---------------------------------------------------------------------------

function resp(status: number, body: BodyInit | null = null, headers: Record<string, string> = {}) {
	return new Response(body, { status, headers });
}
const binary = (b = encode()) =>
	resp(200, b.slice().buffer, { 'content-type': BINARY_TYPE, 'content-length': String(b.length) });

function fake(answers: (() => Response | Promise<Response>)[]) {
	const urls: string[] = [];
	const f: Fetcher = async (url) => {
		urls.push(url);
		const next = answers.shift();
		if (!next) throw new Error('unexpected fetch');
		return next();
	};
	return { f, urls };
}
const noSleep = async () => {};

describe('AnalysisClient', () => {
	it('builds URLs only from plain positive ids', () => {
		expect(analysisUrl(12)).toBe('/v1/analysis/track/12?format=binary');
		expect(analysisUrl('34')).toBe('/v1/analysis/track/34?format=binary');
		for (const bad of [0, -1, 1.5, '01', '../1', '1/2', '1?x', 'abc', null, undefined, 2 ** 60, '1234567890123', {}, NaN]) {
			expect(analysisUrl(bad)).toBeNull();
		}
	});

	it('reads Retry-After within bounds', () => {
		expect(retryAfter('5')).toBe(5);
		expect(retryAfter(null)).toBe(3);
		expect(retryAfter('0')).toBe(1);
		expect(retryAfter('9999')).toBe(30);
		expect(retryAfter('Wed, 21 Oct 2015 07:28:00 GMT')).toBe(3);
	});

	it('fetches once and keeps the result for the session', async () => {
		const { f, urls } = fake([() => binary()]);
		const c = new AnalysisClient(f);
		const [a, b] = await Promise.all([c.load(7), c.load(7)]);
		expect(a.status).toBe('ready');
		expect(b).toBe(a);
		expect(await c.load(7)).toBe(a);
		expect(c.peek(7)).toBe(a);
		expect(urls).toEqual(['/v1/analysis/track/7?format=binary']);
	});

	it('polls a pending analysis, then gives up after the limit', async () => {
		const { f, urls } = fake([() => resp(202, '{"status":"pending"}', { 'retry-after': '2' }), () => resp(429), () => binary()]);
		const slept: number[] = [];
		const c = new AnalysisClient(f, { sleep: async (ms) => void slept.push(ms) });
		expect((await c.load(1)).status).toBe('ready');
		expect(urls).toHaveLength(3);
		expect(slept).toEqual([2000, 3000]);
		const pending = Array.from({ length: 3 }, () => () => resp(202));
		const g = fake(pending);
		const d = new AnalysisClient(g.f, { maxPolls: 2, sleep: noSleep });
		expect((await d.load(1)).status).toBe('unavailable');
		expect(g.urls).toHaveLength(3);
		expect(d.peek(1)).toBeUndefined(); // not kept: asked again next time
	});

	it('keeps refusals, retries transient failures, and learns when analysis is off', async () => {
		const { f, urls } = fake([
			() => resp(422, '{"error":"the audio could not be analysed"}'),
			() => resp(502),
			() => {
				throw new TypeError('network');
			},
			() => binary(),
			() => resp(404, '{"error":"analysis is not available on this server"}'),
		]);
		const c = new AnalysisClient(f, { sleep: noSleep });
		expect((await c.load(1)).status).toBe('unavailable');
		expect((await c.load(1)).status).toBe('unavailable'); // kept: no second fetch
		expect((await c.load(2)).status).toBe('unavailable'); // 502: not kept
		expect((await c.load(2)).status).toBe('unavailable'); // network: not kept
		expect((await c.load(2)).status).toBe('ready');
		expect(c.disabled).toBe(false);
		expect((await c.load(3)).status).toBe('unavailable');
		expect(c.disabled).toBe(true);
		expect((await c.load(4)).status).toBe('unavailable'); // off: not fetched
		expect(urls).toHaveLength(5);
		c.clear();
		expect(c.disabled).toBe(false);
	});

	it('refuses answers that are not the binary form, or are too big, or corrupt', async () => {
		const { f } = fake([
			() => resp(200, '{"version":1}', { 'content-type': 'application/json' }),
			() => resp(200, new Uint8Array(10).buffer, { 'content-type': BINARY_TYPE, 'content-length': String(MAX_BYTES + 1) }),
			() => resp(200, new Uint8Array([1, 2, 3]).buffer, { 'content-type': BINARY_TYPE }),
		]);
		const c = new AnalysisClient(f, { sleep: noSleep });
		for (const id of [1, 2, 3]) expect((await c.load(id)).status).toBe('unavailable');
	});

	it('never fetches for a bad id, and drops the oldest beyond its size', async () => {
		const { f, urls } = fake([() => binary(), () => binary(), () => binary()]);
		const c = new AnalysisClient(f, { maxEntries: 2 });
		expect((await c.load('../../v1/me')).status).toBe('unavailable');
		expect(urls).toHaveLength(0);
		await c.load(1);
		await c.load(2);
		await c.load(3);
		expect(c.peek(1)).toBeUndefined();
		expect(c.peek(3)?.status).toBe('ready');
	});

	it('abandons polling when cleared (sign-out)', async () => {
		const { f } = fake([() => resp(202)]);
		const c = new AnalysisClient(f, {
			sleep: (_ms, signal) =>
				new Promise((_resolve, reject) => signal.addEventListener('abort', () => reject(new Error('aborted')))),
		});
		const p = c.load(1);
		await Promise.resolve();
		await new Promise((r) => setTimeout(r, 0));
		c.clear();
		expect((await p).status).toBe('unavailable');
		expect(c.peek(1)).toBeUndefined();
	});
});

// ---------------------------------------------------------------------------
// The component
// ---------------------------------------------------------------------------

describe('SeekBar', () => {
	beforeAll(() => {
		// jsdom lays nothing out: give elements a width, and a ResizeObserver that reports it.
		Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, get: () => 300 });
		globalThis.ResizeObserver = class {
			#cb: ResizeObserverCallback;
			constructor(cb: ResizeObserverCallback) {
				this.#cb = cb;
			}
			observe(el: Element) {
				this.#cb([{ target: el } as ResizeObserverEntry], this as unknown as ResizeObserver);
			}
			unobserve() {}
			disconnect() {}
		} as unknown as typeof ResizeObserver;
	});

	const settle = async () => {
		for (let i = 0; i < 5; i++) {
			await tick();
			await new Promise((r) => setTimeout(r, 0));
		}
	};

	it('is the plain slider without an analysis', async () => {
		const { f } = fake([() => resp(404)]);
		const client = new AnalysisClient(f);
		const { container } = render(SeekBar, { props: { trackId: 5, value: 30, total: 200, client } });
		await settle();
		const input = container.querySelector('input[type=range]') as HTMLInputElement;
		expect(input).not.toBeNull();
		expect(input.getAttribute('aria-label')).toBe('Position');
		expect(input.getAttribute('aria-valuetext')).toBe('0:30 of 3:20');
		expect(container.querySelector('svg')).toBeNull();
		expect(container.querySelector('.wave')).toBeNull();
	});

	it('draws the waveform with the played part, keeping the slider as the control', async () => {
		const { f } = fake([() => binary(encode({ peak: Array.from({ length: 1000 }, (_, i) => i % 256) }))]);
		const client = new AnalysisClient(f);
		const seeks: number[] = [];
		const { container } = render(SeekBar, {
			props: { trackId: 9, value: 50, total: 200, client, onchange: (v: number) => seeks.push(v) },
		});
		await settle();
		const svg = container.querySelector('svg')!;
		expect(svg).not.toBeNull();
		expect(svg.getAttribute('aria-hidden')).toBe('true');
		expect(container.querySelectorAll('path')).toHaveLength(2);
		const clip = container.querySelector('clipPath rect')!;
		expect(Number(clip.getAttribute('width'))).toBeCloseTo(75); // a quarter of 300 px
		const input = container.querySelector('input[type=range]') as HTMLInputElement;
		input.value = '120';
		input.dispatchEvent(new Event('change', { bubbles: true }));
		expect(seeks).toEqual([120]);
	});

	it('shows nothing extra for a stream (no id) and fetches nothing', async () => {
		const { f, urls } = fake([]);
		const client = new AnalysisClient(f);
		const { container } = render(SeekBar, { props: { trackId: null, value: 0, total: 0, disabled: true, client } });
		await settle();
		expect(container.querySelector('svg')).toBeNull();
		expect((container.querySelector('input') as HTMLInputElement).disabled).toBe(true);
		expect(urls).toHaveLength(0);
	});
});
