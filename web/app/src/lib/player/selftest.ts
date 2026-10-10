// Media-element self-test: which of our segment variants this browser really plays.
//
// Advertised support (`isTypeSupported`) isn't enough: iPadOS Safari accepts the FLAC type,
// then rejects the stream itself. So, once per browser version, the app builds a few tiny
// test streams (two sample rates, a few header variants), appends them to a Media Source, and
// records which ones the browser accepts. The playback mode and header variant come from
// these results, and they are reported in diagnostics.

import type { Capabilities } from './capabilities';
import { mediaFormat } from './mode';

/** The Media Source API and MIME spelling to self-test, if the browser advertises any. */
export const mediaFormatFor = (caps: Capabilities) => mediaFormat(caps);

export interface Probe {
	rate: number;
	/** Variant code for the core's segmenter (brand bits 0–3, high-rate style bits 4–7). */
	code: number;
}

export interface ProbeResult extends Probe {
	ok: boolean;
	error?: string;
}

export interface SelfTest {
	version: number;
	userAgent: string;
	source: 'managed' | 'classic';
	mime: string;
	results: ProbeResult[];
}

/** Bump when the probes or the segmenter change, so cached results are redone. */
export const SELFTEST_VERSION = 2;

/** Low rate first (every track needs it), then ways to write rates above 65,535 Hz. */
export const PROBES: Probe[] = [
	{ rate: 44_100, code: 0x00 },
	{ rate: 44_100, code: 0x01 },
	{ rate: 96_000, code: 0x00 },
	{ rate: 96_000, code: 0x10 },
	{ rate: 96_000, code: 0x20 },
	{ rate: 96_000, code: 0x11 },
];

/** Pick the variant to use: one that plays 44.1 kHz and, if possible, high rates too. */
export function chooseVariant(t: SelfTest | null): { media: boolean; code: number; highRates: boolean } {
	if (!t) return { media: false, code: 0, highRates: false };
	const ok = (rate: number, code: number) => t.results.some((r) => r.rate === rate && r.code === code && r.ok);
	const brands = [0x0, 0x1, 0x2];
	for (const code of [0x00, 0x10, 0x20, 0x01, 0x11, 0x21]) {
		if (ok(44_100, code & 0xf) && ok(96_000, code)) return { media: true, code, highRates: true };
	}
	const low = brands.find((b) => ok(44_100, b));
	if (low !== undefined) return { media: true, code: low, highRates: false };
	return { media: false, code: 0, highRates: false };
}

const KEY = 'anywhere.mse.selftest';

export function loadCached(ua: string, storage: Pick<Storage, 'getItem'> | undefined = safeLocal()): SelfTest | null {
	try {
		const t = JSON.parse(storage?.getItem(KEY) ?? 'null') as SelfTest | null;
		if (!t || t.version !== SELFTEST_VERSION || t.userAgent !== ua || !Array.isArray(t.results)) return null;
		return t;
	} catch {
		return null;
	}
}

export function saveCached(t: SelfTest, storage: Pick<Storage, 'setItem'> | undefined = safeLocal()) {
	try {
		storage?.setItem(KEY, JSON.stringify(t));
	} catch {
		/* not essential */
	}
}

function safeLocal(): Storage | undefined {
	try {
		return globalThis.localStorage;
	} catch {
		return undefined;
	}
}

type Segmenter = {
	init_segment(): Uint8Array;
	push(pcm: Float32Array): void;
	flush(): Uint8Array | undefined;
	set_variant(code: number): void;
	free?: () => void;
};
export type MakeSegmenter = (rate: number) => Segmenter;

const wait = <T>(p: Promise<T>, ms: number, what: string) =>
	Promise.race([p, new Promise<T>((_, rej) => setTimeout(() => rej(new Error(`${what} timed out`)), ms))]);

/** Try one variant: build ~0.2 s of silence and see if the browser buffers it without error. */
export async function probe(p: Probe, source: 'managed' | 'classic', mime: string, make: MakeSegmenter): Promise<ProbeResult> {
	const g = globalThis as unknown as { ManagedMediaSource?: new () => MediaSource; MediaSource?: new () => MediaSource };
	const Ctor = source === 'managed' ? g.ManagedMediaSource : g.MediaSource;
	if (!Ctor) return { ...p, ok: false, error: 'no Media Source' };
	const audio = new Audio();
	(audio as HTMLAudioElement & { disableRemotePlayback: boolean }).disableRemotePlayback = true;
	audio.muted = true;
	const ms = new Ctor();
	const url = URL.createObjectURL(ms);
	let failed = '';
	audio.addEventListener('error', () => (failed = `element error ${audio.error?.code ?? ''} ${audio.error?.message ?? ''}`.trim()));
	try {
		audio.src = url;
		await wait(new Promise<void>((r) => ms.addEventListener('sourceopen', () => r(), { once: true })), 3000, 'sourceopen');
		const sb = ms.addSourceBuffer(mime);
		const append = (b: Uint8Array) =>
			wait(
				new Promise<void>((resolve, reject) => {
					sb.addEventListener('updateend', () => resolve(), { once: true });
					sb.addEventListener('error', () => reject(new Error('buffer error')), { once: true });
					sb.appendBuffer(b as BufferSource);
				}),
				3000,
				'append',
			);
		const seg = make(p.rate);
		seg.set_variant(p.code);
		await append(seg.init_segment());
		seg.push(new Float32Array(Math.round(p.rate * 0.2) * 2));
		const media = seg.flush();
		seg.free?.();
		if (!media) throw new Error('no media segment');
		await append(media);
		await new Promise((r) => setTimeout(r, 150)); // give a late decode error time to surface
		if (failed) throw new Error(failed);
		if (!sb.buffered.length || sb.buffered.end(0) < 0.1) throw new Error('nothing buffered');
		return { ...p, ok: true };
	} catch (e) {
		return { ...p, ok: false, error: failed || (e as Error).message };
	} finally {
		try {
			if (ms.readyState === 'open') ms.endOfStream();
		} catch {
			/* closing anyway */
		}
		audio.removeAttribute('src');
		audio.load();
		URL.revokeObjectURL(url);
	}
}

export async function runSelfTest(source: 'managed' | 'classic', mime: string, ua: string, make: MakeSegmenter): Promise<SelfTest> {
	const results: ProbeResult[] = [];
	for (const p of PROBES) results.push(await probe(p, source, mime, make));
	return { version: SELFTEST_VERSION, userAgent: ua, source, mime, results };
}
