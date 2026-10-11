// Fetching track analyses (GET /v1/analysis/track/{id}?format=binary) for the waveform seek
// bar, cached for the session.
//
// - 200: decoded (and checked) → ready.
// - 202: the gateway is still analysing → pending; asked again after Retry-After (1–30 s), a
//   bounded number of times.
// - 429: busy → retried like 202.
// - 404, 403, 413, 415, 422, 504: no analysis for this track → unavailable for the session.
//   "analysis is not available on this server" turns fetching off for the session.
// - Network trouble and 5xx from LMS: unavailable now, tried again next time.
import { BINARY_TYPE, MAX_BYTES, decodeAnalysis, type TrackAnalysis } from './format';

export type AnalysisState =
	| { status: 'pending' }
	| { status: 'ready'; analysis: TrackAnalysis }
	| { status: 'unavailable' };

/** Performs the authenticated GET (the caller supplies the token handling). */
export type Fetcher = (url: string, signal: AbortSignal) => Promise<Response>;

export interface ClientOptions {
	/** Results kept (oldest dropped first). */
	maxEntries?: number;
	/** Times a pending analysis is asked for again. */
	maxPolls?: number;
	sleep?: (ms: number, signal: AbortSignal) => Promise<void>;
}

const NOT_AVAILABLE = 'analysis is not available on this server';

/** The URL for a track id, or null for anything but a positive integer id. */
export function analysisUrl(id: unknown): string | null {
	const s = typeof id === 'number' ? (Number.isSafeInteger(id) ? String(id) : '') : typeof id === 'string' ? id : '';
	if (!/^[1-9][0-9]{0,11}$/.test(s)) return null;
	return `/v1/analysis/track/${s}?format=binary`;
}

/** Seconds from a Retry-After header, clamped to 1–30 (3 when missing or malformed). */
export function retryAfter(v: string | null): number {
	const n = v !== null && /^\d{1,6}$/.test(v.trim()) ? Number(v.trim()) : 3;
	return Math.min(30, Math.max(1, n));
}

function defaultSleep(ms: number, signal: AbortSignal): Promise<void> {
	return new Promise((resolve, reject) => {
		if (signal.aborted) return reject(new DOMException('aborted', 'AbortError'));
		const t = setTimeout(resolve, ms);
		signal.addEventListener(
			'abort',
			() => {
				clearTimeout(t);
				reject(new DOMException('aborted', 'AbortError'));
			},
			{ once: true },
		);
	});
}

export class AnalysisClient {
	#fetch: Fetcher;
	#results = new Map<string, AnalysisState>();
	#inflight = new Map<string, Promise<AnalysisState>>();
	#off = false;
	#abort = new AbortController();
	#max: number;
	#polls: number;
	#sleep: (ms: number, signal: AbortSignal) => Promise<void>;

	constructor(fetcher: Fetcher, opts: ClientOptions = {}) {
		this.#fetch = fetcher;
		this.#max = Math.max(1, opts.maxEntries ?? 300);
		this.#polls = Math.max(0, opts.maxPolls ?? 40);
		this.#sleep = opts.sleep ?? defaultSleep;
	}

	/** The server has no analysis service (learnt from an answer). */
	get disabled(): boolean {
		return this.#off;
	}

	/** A finished result already known, if any. */
	peek(id: unknown): AnalysisState | undefined {
		const url = analysisUrl(id);
		return url ? this.#results.get(url) : { status: 'unavailable' };
	}

	/** The analysis of a track: from the cache, or fetched (polling while it is pending). */
	load(id: unknown): Promise<AnalysisState> {
		const url = analysisUrl(id);
		if (!url || this.#off) return Promise.resolve({ status: 'unavailable' });
		const hit = this.#results.get(url);
		if (hit) {
			this.#results.delete(url);
			this.#results.set(url, hit); // most recently used last
			return Promise.resolve(hit);
		}
		const running = this.#inflight.get(url);
		if (running) return running;
		const p = this.#run(url, this.#abort.signal).finally(() => this.#inflight.delete(url));
		this.#inflight.set(url, p);
		return p;
	}

	/** Forget everything (sign-out, or another account). */
	clear() {
		this.#abort.abort();
		this.#abort = new AbortController();
		this.#results.clear();
		this.#inflight.clear();
		this.#off = false;
	}

	#keep(url: string, s: AnalysisState): AnalysisState {
		this.#results.set(url, s);
		while (this.#results.size > this.#max) {
			const oldest = this.#results.keys().next().value!;
			this.#results.delete(oldest);
		}
		return s;
	}

	async #run(url: string, signal: AbortSignal): Promise<AnalysisState> {
		const gone: AnalysisState = { status: 'unavailable' };
		for (let attempt = 0; attempt <= this.#polls; attempt++) {
			let resp: Response;
			try {
				resp = await this.#fetch(url, signal);
			} catch {
				return gone; // not kept: tried again next time
			}
			if (resp.status === 200) {
				const type = (resp.headers.get('content-type') ?? '').split(';')[0]!.trim();
				const len = Number(resp.headers.get('content-length') ?? '0');
				if (type !== BINARY_TYPE || len > MAX_BYTES) return this.#keep(url, gone);
				let buf: ArrayBuffer;
				try {
					buf = await resp.arrayBuffer();
				} catch {
					return gone;
				}
				const analysis = decodeAnalysis(buf);
				return this.#keep(url, analysis ? { status: 'ready', analysis } : gone);
			}
			if (resp.status === 202 || resp.status === 429) {
				if (attempt === this.#polls) return gone;
				try {
					await this.#sleep(retryAfter(resp.headers.get('retry-after')) * 1000, signal);
				} catch {
					return gone;
				}
				continue;
			}
			if (resp.status === 404) {
				const body = (await resp.json().catch(() => ({}))) as { error?: unknown };
				if (body?.error === NOT_AVAILABLE) this.#off = true;
				return this.#keep(url, gone);
			}
			if ([403, 413, 415, 422, 504].includes(resp.status)) return this.#keep(url, gone);
			return gone; // 5xx and the unexpected: tried again next time
		}
		return gone;
	}
}
