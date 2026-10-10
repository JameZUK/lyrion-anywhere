// Choosing how "this device" plays audio.
//
// - Web Audio ('worklet'): decoded PCM straight into an AudioWorklet. Lowest latency and
//   sample-accurate; desktop and Android browsers keep it running in the background.
// - Media element ('media'): the same PCM, losslessly re-wrapped as FLAC in fragmented MP4
//   and fed to an <audio> element through (Managed) Media Source. iOS/iPadOS suspend Web
//   Audio when the page is in the background, but keep a playing media element going.
import type { Capabilities } from './capabilities';

export type PlaybackMode = 'worklet' | 'media';
export type ModePreference = 'auto' | PlaybackMode;
/**
 * What the media-element path appends: FLAC in fragmented MP4. Browsers disagree on the
 * codec name, and the check is case-sensitive: Chromium accepts "flac", Safari only "fLaC".
 */
export const MEDIA_TYPES = ['audio/mp4; codecs="flac"', 'audio/mp4; codecs="fLaC"'] as const;
/** The first spelling (kept for callers that only need one). */
export const MEDIA_TYPE = MEDIA_TYPES[0];

export interface Environment {
	/** iPhone, iPod or iPad (iPadOS reports itself as a Mac, so touch support decides). */
	appleMobile: boolean;
	/** Android phones and tablets. */
	android?: boolean;
}

export interface ModeChoice {
	mode: PlaybackMode;
	/** For 'media': which Media Source API to use. */
	source: 'managed' | 'classic' | null;
	/** For 'media': the MIME type this browser accepts for our segments. */
	mime?: string | null;
	/** Why, for Settings. */
	reason: string;
}

export function detectEnvironment(nav: { userAgent?: string; maxTouchPoints?: number } = globalThis.navigator ?? {}): Environment {
	const ua = nav.userAgent ?? '';
	const ipadAsMac = /Macintosh/.test(ua) && (nav.maxTouchPoints ?? 0) > 1;
	return { appleMobile: /iPhone|iPad|iPod/.test(ua) || ipadAsMac, android: /Android/.test(ua) };
}

/** Which Media Source API and MIME spelling can play our segments (Managed preferred). */
export function mediaFormat(caps: Capabilities): { source: 'managed' | 'classic'; mime: string } | null {
	for (const source of ['managed', 'classic'] as const) {
		if (source === 'managed' ? !caps.managedMediaSource : !caps.mediaSource) continue;
		for (const mime of MEDIA_TYPES) {
			const t = caps.types.find((x) => x.type === mime);
			if (t && (source === 'managed' ? t.managed : t.classic)) return { source, mime };
		}
	}
	return null;
}

/** Which Media Source API can play our segments, preferring Managed (needed on iPhone). */
export function mediaSource(caps: Capabilities): 'managed' | 'classic' | null {
	return mediaFormat(caps)?.source ?? null;
}

export function chooseMode(caps: Capabilities, env: Environment, pref: ModePreference = 'auto'): ModeChoice {
	const c = chooseModeInner(caps, env, pref);
	return c.mode === 'media' && c.source ? { ...c, mime: mediaFormat(caps)?.mime ?? null } : c;
}

function chooseModeInner(caps: Capabilities, env: Environment, pref: ModePreference): ModeChoice {
	const source = mediaSource(caps);
	const worklet = caps.audioWorklet;
	if (pref === 'media') {
		if (source) return { mode: 'media', source, reason: 'Chosen in Settings.' };
		return worklet
			? { mode: 'worklet', source: null, reason: 'Media element chosen in Settings, but this browser cannot play the stream that way; using Web Audio.' }
			: { mode: 'media', source: null, reason: 'This browser supports neither playback method.' };
	}
	if (pref === 'worklet') {
		if (worklet) return { mode: 'worklet', source: null, reason: 'Chosen in Settings.' };
		if (source) return { mode: 'media', source, reason: 'Web Audio chosen in Settings, but this browser lacks it; using the media element.' };
		return { mode: 'worklet', source: null, reason: 'This browser supports neither playback method.' };
	}
	// Automatic.
	if (env.appleMobile && source) {
		return { mode: 'media', source, reason: 'iPhone/iPad: only a media element keeps playing when you switch apps or lock the screen.' };
	}
	if (env.android && source) {
		return { mode: 'media', source, reason: 'Android: a media element keeps playing in the background, with media controls in the notification.' };
	}
	if (worklet) {
		return {
			mode: 'worklet',
			source: null,
			reason: env.appleMobile
				? 'iPhone/iPad without lossless Media Source support: Web Audio, which may pause in the background.'
				: 'Web Audio: lowest latency, and this browser keeps it playing in the background.',
		};
	}
	if (source) return { mode: 'media', source, reason: 'No Web Audio worklets in this browser: using the media element.' };
	return { mode: 'worklet', source: null, reason: 'This browser supports neither playback method.' };
}

/** The user's preference, kept per browser. */
export function loadPreference(storage: Pick<Storage, 'getItem'> | undefined = safeLocal()): ModePreference {
	try {
		const v = storage?.getItem('anywhere.playback.mode');
		return v === 'worklet' || v === 'media' ? v : 'auto';
	} catch {
		return 'auto';
	}
}

export function savePreference(pref: ModePreference, storage: Pick<Storage, 'setItem' | 'removeItem'> | undefined = safeLocal()) {
	try {
		if (pref === 'auto') storage?.removeItem('anywhere.playback.mode');
		else storage?.setItem('anywhere.playback.mode', pref);
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

export const MODE_LABELS: Record<PlaybackMode, string> = {
	worklet: 'Web Audio',
	media: 'Media element',
};
