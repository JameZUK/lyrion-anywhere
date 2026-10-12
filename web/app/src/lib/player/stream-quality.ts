// Streaming quality for this browser's player (docs/plans/streaming-quality.md): the level
// asked of the gateway, chosen in Settings or automatically from the network.

export type StreamQuality = 'original' | 'high' | 'low';
export type QualityPreference = 'auto' | StreamQuality;

export const PREFERENCES: QualityPreference[] = ['auto', 'original', 'high', 'low'];

/** What the Network Information API says, where the browser has it (not Safari). */
export interface ConnectionInfo {
	type?: string;
	saveData?: boolean;
}

/**
 * The level for "Automatic": Data saver when the browser is asked to save data, High on a
 * mobile connection, otherwise Original. Without the API (Safari, so iPhone and iPad), or
 * when it doesn't say, Original.
 */
export function autoQuality(c: ConnectionInfo | null | undefined): StreamQuality {
	if (!c) return 'original';
	if (c.saveData === true) return 'low';
	if (c.type === 'cellular') return 'high';
	return 'original';
}

export function effectiveQuality(pref: QualityPreference, c: ConnectionInfo | null | undefined): StreamQuality {
	return pref === 'auto' ? autoQuality(c) : pref;
}

const KEY = 'anywhere.stream.quality';
type Store = Pick<Storage, 'getItem' | 'setItem'>;
function safeLocal(): Store | undefined {
	try {
		return globalThis.localStorage;
	} catch {
		return undefined;
	}
}

export function sanitisePreference(v: unknown): QualityPreference {
	return PREFERENCES.includes(v as QualityPreference) ? (v as QualityPreference) : 'auto';
}

/** The preference kept in this browser ("Automatic" unless changed). */
export function loadPreference(store: Store | undefined = safeLocal()): QualityPreference {
	try {
		return sanitisePreference(store?.getItem(KEY));
	} catch {
		return 'auto';
	}
}

export function savePreference(p: QualityPreference, store: Store | undefined = safeLocal()) {
	try {
		store?.setItem(KEY, sanitisePreference(p));
	} catch {
		/* not essential */
	}
}

/** The browser's connection object, if it has one. */
export function connection(nav: unknown = globalThis.navigator): (ConnectionInfo & EventTarget) | null {
	const c = (nav as { connection?: unknown } | undefined)?.connection;
	return c && typeof c === 'object' ? (c as ConnectionInfo & EventTarget) : null;
}
