// Diagnostics this device reports to the gateway, for troubleshooting by an admin (LMS ›
// Settings › Plugins › Lyrion Anywhere › Devices › Diagnostics). Only playback-related facts:
// browser capabilities, the chosen playback mode and why, player status and the recent
// connection log. Sent batched, and at once when the page goes into the background.
import type { Capabilities } from './capabilities';
import type { Environment, ModeChoice, ModePreference } from './mode';
import { ApiError, CLIENT_HEADER, bearer } from '#lib/api/http';
import type { Command } from '#lib/api/rpc';

/**
 * Sends a report to the gateway through `authed`: a refused (expired) token surfaces as an
 * ApiError 401, so `authed` renews it and tries once more; any other failure is an error too.
 */
export function diagnosticsSender(
	authed: <T>(fn: (token: string) => Promise<T>) => Promise<T>,
	doFetch: typeof fetch = (...a) => fetch(...a),
) {
	return (body: string, urgent: boolean) =>
		authed(async (token) => {
			const resp = await doFetch('/v1/diagnostics', {
				method: 'POST',
				headers: { ...CLIENT_HEADER, ...bearer(token), 'Content-Type': 'application/json' },
				body,
				keepalive: urgent,
			});
			if (!resp.ok) throw new ApiError(resp.status, `diagnostics HTTP ${resp.status}`);
		});
}

export interface ReportInput {
	capabilities: Capabilities;
	environment: Environment;
	preference: ModePreference;
	planned: ModeChoice;
	active: ModeChoice | null;
	device: { state: string; mac: string; name: string; detail: Record<string, unknown> };
	log: string[];
	page: { visibility: string; standalone: boolean };
	/** Which media-element variants the browser accepted. */
	selftest?: unknown;
}

/**
 * The newest log lines a report carries, and at most this many bytes of them: the gateway
 * takes a report of up to 32 KiB, and the rest of it is small.
 */
export const LOG_LINES = 120;
export const LOG_BYTES = 16 * 1024;

/** The newest lines (first in `log`) that fit {@link LOG_LINES} and {@link LOG_BYTES}. */
export function recentLog(log: string[]): string[] {
	const out: string[] = [];
	let bytes = 0;
	for (const line of log.slice(0, LOG_LINES)) {
		bytes += new TextEncoder().encode(line).length + 3; // quotes and comma in the JSON
		if (bytes > LOG_BYTES) break;
		out.push(line);
	}
	return out;
}

export function buildReport(i: ReportInput, now = new Date()) {
	return {
		app: 'web',
		at: now.toISOString(),
		userAgent: i.capabilities.userAgent,
		environment: i.environment,
		capabilities: {
			managedMediaSource: i.capabilities.managedMediaSource,
			mediaSource: i.capabilities.mediaSource,
			audioWorklet: i.capabilities.audioWorklet,
			audioSession: i.capabilities.audioSession,
			types: i.capabilities.types,
		},
		playback: { preference: i.preference, planned: i.planned, active: i.active },
		device: i.device,
		page: i.page,
		selftest: i.selftest ?? null,
		log: recentLog(i.log),
	};
}

/** A report's content for "anything new?": without its time and the page's visibility. */
function contentOf(report: object): string {
	const { at: _at, page: _page, ...rest } = report as Record<string, unknown>;
	return JSON.stringify(rest);
}

/**
 * Sends at most one report per `minGap` ms, except urgent ones (page going away). An urgent
 * one with nothing new since the last report that got through is not sent (hiding the tab
 * over and over used to post a report every time).
 */
export class Reporter {
	#timer: ReturnType<typeof setTimeout> | undefined;
	#lastSent = -Infinity; // the first report goes at once
	/** The content of the last report that got through ('' before any). */
	#lastContent = '';

	constructor(
		private send: (body: string, urgent: boolean) => Promise<void>,
		private make: () => object,
		private minGap = 15_000,
		private now = () => Date.now(),
	) {}

	/** Report soon (batched). */
	schedule() {
		if (this.#timer) return;
		const wait = Math.max(0, this.#lastSent + this.minGap - this.now());
		this.#timer = setTimeout(() => {
			this.#timer = undefined;
			void this.#go(false);
		}, wait);
	}

	/** Report now (e.g. the page is being hidden). */
	urgent() {
		const pending = !!this.#timer;
		clearTimeout(this.#timer);
		this.#timer = undefined;
		void this.#go(true, !pending);
	}

	async #go(urgent: boolean, onlyIfNew = false) {
		const report = this.make();
		const content = contentOf(report);
		if (onlyIfNew && content === this.#lastContent) return;
		this.#lastSent = this.now();
		// Noted before the send, so two hides in quick succession post once; undone if it fails.
		const before = this.#lastContent;
		this.#lastContent = content;
		try {
			await this.send(JSON.stringify(report), urgent);
		} catch {
			/* diagnostics are best effort: the next hide tries again */
			if (this.#lastContent === content) this.#lastContent = before;
		}
	}
}

/** No kind of rate-limited log line is logged more often than this (ms). */
export const LOG_KIND_EVERY_MS = 5_000;
/** Kinds remembered at once; the oldest is forgotten first. */
const LOG_KINDS = 64;

/**
 * Keeps the connection log readable (issue #12 diagnostics): a line of a `kind` (such as
 * 'media:waiting' or 'stat:STMo') is logged at most once per {@link LOG_KIND_EVERY_MS}; the
 * next one logged says how many were left out meanwhile. Lines without a kind (state
 * changes, and everything logged before kinds existed) always go through.
 */
export class LogLimiter {
	#kinds = new Map<string, { at: number; skipped: number }>();

	constructor(private every = LOG_KIND_EVERY_MS) {}

	/** The line to log for `msg` at `now` (ms), or null to leave it out. */
	pass(msg: string, kind: string | undefined, now: number): string | null {
		if (!kind) return msg;
		const seen = this.#kinds.get(kind);
		if (seen && now - seen.at < this.every && now >= seen.at) {
			seen.skipped++;
			return null;
		}
		const skipped = seen?.skipped ?? 0;
		this.#kinds.delete(kind);
		this.#kinds.set(kind, { at: now, skipped: 0 });
		if (this.#kinds.size > LOG_KINDS) this.#kinds.delete(this.#kinds.keys().next().value!);
		return skipped ? `${msg} (and ${skipped} more like it)` : msg;
	}
}

/** What a command the page sends LMS asks for, in a few words (never its free text). */
export function describeCommand(c: Command): string {
	const [a, b, d] = c;
	const num = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? Math.round(v * 10) / 10 : typeof v === 'string' && /^-?\d+(\.\d+)?$/.test(v) ? Number(v) : null);
	if (a === 'pause') return b === 1 || b === '1' ? 'pause' : b === 0 || b === '0' ? 'resume' : 'pause or resume';
	if (a === 'play' || a === 'stop') return a;
	if (a === 'time' && num(b) !== null) return `seek to ${num(b)} s`;
	if (a === 'playlist' && b === 'index' && num(d) !== null) return `play queue entry ${num(d)}`;
	if (a === 'mixer' && b === 'volume' && num(d) !== null) return `set the volume to ${num(d)}%`;
	if (a === 'playerpref' && b === 'digitalVolumeControl') {
		if (d === '0' || d === 0) return 'fix the output level at 100%';
		if (d === '1' || d === 1) return 'let LMS set the volume';
		if (d === '?') return 'report its Volume Control';
	}
	return typeof a === 'string' && /^[a-z]{1,20}$/.test(a) ? `run '${a}'` : 'run a command';
}

/**
 * A command the page sends LMS on its own (not a tap on a control), for the connection log:
 * `page: asked LMS to pause (the system paused the media element)`.
 */
export function describePageCommand(c: Command, why: string): string {
	return `page: asked LMS to ${describeCommand(c)} (${why})`;
}
