// What this browser can do for audio output. Read-only feature detection, shown in Settings
// so a device (e.g. an iPad) can tell us which background-capable path it supports.

export const MSE_TYPES = [
	'audio/mp4; codecs="flac"',
	'audio/mp4; codecs="fLaC"',
	'audio/mp4; codecs="opus"',
	'audio/mp4; codecs="mp4a.40.2"',
	'audio/mp4; codecs="alac"',
	'audio/mp4; codecs="lpcm"',
	'audio/webm; codecs="opus"',
	'audio/mpeg',
	'audio/aac',
] as const;

export interface Capabilities {
	managedMediaSource: boolean;
	mediaSource: boolean;
	audioWorklet: boolean;
	audioSession: boolean;
	/** For each MSE type: supported by ManagedMediaSource / MediaSource. */
	types: { type: string; managed: boolean; classic: boolean }[];
	userAgent: string;
}

type MseCtor = { isTypeSupported(t: string): boolean } | undefined;

export function detect(g: Record<string, unknown> = globalThis as unknown as Record<string, unknown>): Capabilities {
	const mms = g.ManagedMediaSource as MseCtor;
	const ms = g.MediaSource as MseCtor;
	const test = (c: MseCtor, t: string) => {
		try {
			return !!c && c.isTypeSupported(t);
		} catch {
			return false;
		}
	};
	const nav = (g.navigator ?? {}) as { userAgent?: string; audioSession?: unknown };
	return {
		managedMediaSource: !!mms,
		mediaSource: !!ms,
		audioWorklet: typeof g.AudioWorkletNode !== 'undefined',
		audioSession: !!nav.audioSession,
		types: MSE_TYPES.map((t) => ({ type: t, managed: test(mms, t), classic: test(ms, t) })),
		userAgent: String(nav.userAgent ?? '').slice(0, 300),
	};
}

/** Can this browser be a player at all (an audio output, a worker and WebAssembly)? */
export function canPlayHere(g: Record<string, unknown> = globalThis as unknown as Record<string, unknown>): boolean {
	const output = typeof g.AudioWorkletNode !== 'undefined' || !!g.ManagedMediaSource || !!g.MediaSource;
	return output && typeof g.Worker !== 'undefined' && typeof g.WebAssembly !== 'undefined';
}
