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

/**
 * Whether Web Audio worklets can run here: the `AudioWorkletNode` constructor *and* an
 * `audioWorklet` on audio contexts. Pages on plain `http://` (anything but localhost) are
 * insecure contexts, where browsers withhold `audioWorklet`; Chromium still exposes the
 * `AudioWorkletNode` constructor there, so testing that alone sent the player down a path
 * that then failed. Read from the prototype: no AudioContext is made just to look.
 */
export function hasAudioWorklet(g: Record<string, unknown> = globalThis as unknown as Record<string, unknown>): boolean {
	try {
		if (typeof g.AudioWorkletNode !== 'function') return false;
		const ctor = (g.AudioContext ?? g.webkitAudioContext) as { prototype?: unknown } | undefined;
		const proto = typeof ctor === 'function' ? ctor.prototype : undefined;
		return !!proto && typeof proto === 'object' && 'audioWorklet' in proto;
	} catch {
		return false;
	}
}

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
		audioWorklet: hasAudioWorklet(g),
		audioSession: !!nav.audioSession,
		types: MSE_TYPES.map((t) => ({ type: t, managed: test(mms, t), classic: test(ms, t) })),
		userAgent: String(nav.userAgent ?? '').slice(0, 300),
	};
}

/** Can this browser be a player at all (an audio output, a worker and WebAssembly)? */
export function canPlayHere(g: Record<string, unknown> = globalThis as unknown as Record<string, unknown>): boolean {
	const output = hasAudioWorklet(g) || !!g.ManagedMediaSource || !!g.MediaSource;
	return output && typeof g.Worker !== 'undefined' && typeof g.WebAssembly !== 'undefined';
}
