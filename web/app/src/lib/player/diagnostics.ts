// Diagnostics this device reports to the gateway, for troubleshooting by an admin (LMS ›
// Settings › Plugins › Lyrion Anywhere › Devices › Diagnostics). Only playback-related facts:
// browser capabilities, the chosen playback mode and why, player status and the recent
// connection log. Sent batched, and at once when the page goes into the background.
import type { Capabilities } from './capabilities';
import type { Environment, ModeChoice, ModePreference } from './mode';

export interface ReportInput {
	capabilities: Capabilities;
	environment: Environment;
	preference: ModePreference;
	planned: ModeChoice;
	active: ModeChoice | null;
	device: { state: string; mac: string; name: string; detail: Record<string, unknown> };
	log: string[];
	page: { visibility: string; standalone: boolean };
}

const LOG_LINES = 60;

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
		log: i.log.slice(0, LOG_LINES),
	};
}

/** Sends at most one report per `minGap` ms, except urgent ones (page going away). */
export class Reporter {
	#timer: ReturnType<typeof setTimeout> | undefined;
	#lastSent = -Infinity; // the first report goes at once

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
		clearTimeout(this.#timer);
		this.#timer = undefined;
		void this.#go(true);
	}

	async #go(urgent: boolean) {
		this.#lastSent = this.now();
		try {
			await this.send(JSON.stringify(this.make()), urgent);
		} catch {
			/* diagnostics are best effort */
		}
	}
}
