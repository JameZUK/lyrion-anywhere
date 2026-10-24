// Download preferences (SPEC D5, §8.2, §8.3): a default format for Wi-Fi and one for mobile
// data, "download on Wi-Fi only", a storage limit, and this device's storage use. Kept in
// local storage; never anything secret.
import { isFormat, type Format } from './types';

export interface DownloadPrefs {
	wifi: Format;
	cellular: Format;
	/** On mobile data, downloads wait for Wi-Fi (only where the browser reports the network). */
	wifiOnly: boolean;
	/** Most bytes downloads may use on this device; 0 is no limit. */
	capBytes: number;
}

const GB = 1024 ** 3;
/** The storage limits offered (0: no limit), as on Android. */
export const CAP_CHOICES = [0, 1 * GB, 2 * GB, 5 * GB, 10 * GB, 20 * GB, 50 * GB, 100 * GB] as const;
export const MAX_CAP = 1024 * GB;

export const DEFAULT_PREFS: DownloadPrefs = { wifi: 'original', cellular: 'opus', wifiOnly: true, capBytes: 0 };

/** A storage limit from storage: a whole number of bytes up to [`MAX_CAP`], else none. */
const cap = (v: unknown) => (typeof v === 'number' && Number.isSafeInteger(v) && v > 0 && v <= MAX_CAP ? v : 0);
const PREFS_KEY = 'anywhere.downloads';

type Get = Pick<Storage, 'getItem'>;
type Set = Pick<Storage, 'setItem'>;

function local(): Storage | undefined {
	try {
		return globalThis.localStorage;
	} catch {
		return undefined;
	}
}

export function loadPrefs(storage: Get | undefined = local()): DownloadPrefs {
	try {
		const v = JSON.parse(storage?.getItem(PREFS_KEY) ?? 'null') as Partial<DownloadPrefs> | null;
		return {
			wifi: isFormat(v?.wifi) ? v.wifi : DEFAULT_PREFS.wifi,
			cellular: isFormat(v?.cellular) ? v.cellular : DEFAULT_PREFS.cellular,
			wifiOnly: typeof v?.wifiOnly === 'boolean' ? v.wifiOnly : DEFAULT_PREFS.wifiOnly,
			capBytes: cap(v?.capBytes),
		};
	} catch {
		return { ...DEFAULT_PREFS };
	}
}

export function savePrefs(p: DownloadPrefs, storage: Set | undefined = local()) {
	try {
		storage?.setItem(PREFS_KEY, JSON.stringify({ wifi: p.wifi, cellular: p.cellular, wifiOnly: p.wifiOnly, capBytes: cap(p.capBytes) }));
	} catch {
		/* storage blocked: the defaults apply */
	}
}

export type Network = 'wifi' | 'cellular';

interface Connection {
	type?: string;
	effectiveType?: string;
	saveData?: boolean;
}

/**
 * Which default applies now. The Network Information API is only in Chromium-based
 * browsers (and not on iOS): `type` says "cellular" on Android; where only `effectiveType`
 * is known, a slow connection (2G/3G) or Data Saver is treated like mobile data. Anything
 * else, including no API at all, uses the Wi-Fi default.
 */
export function networkType(nav: { connection?: Connection } | undefined = globalThis.navigator as never): Network {
	const c = nav?.connection;
	if (!c) return 'wifi';
	if (c.type === 'cellular') return 'cellular';
	if (c.type && c.type !== 'unknown') return 'wifi';
	if (c.saveData) return 'cellular';
	return c.effectiveType && /^(slow-2g|2g|3g)$/.test(c.effectiveType) ? 'cellular' : 'wifi';
}

/**
 * What the browser says about the link, for "download on Wi-Fi only": only `type` tells
 * mobile data from Wi-Fi (Chromium on Android has it; Safari, Firefox and desktop Chrome
 * don't). `effectiveType` is about speed, not cost, so it never holds a download back.
 */
export function connectionKind(nav: { connection?: Connection } | undefined = globalThis.navigator as never): Network | 'unknown' {
	const t = nav?.connection?.type;
	if (t === 'cellular') return 'cellular';
	if (t === 'wifi' || t === 'ethernet' || t === 'wimax') return 'wifi';
	return 'unknown';
}

/** Whether this browser reports the network type at all. */
export const reportsNetwork = (nav: { connection?: Connection } | undefined = globalThis.navigator as never): boolean =>
	typeof nav?.connection?.type === 'string';

/** Downloads wait now: Wi-Fi only is on and the browser says this is mobile data. */
export const waitForWifi = (prefs: DownloadPrefs = loadPrefs(), nav?: { connection?: Connection }): boolean =>
	prefs.wifiOnly && connectionKind(nav ?? (globalThis.navigator as never)) === 'cellular';

export const defaultFormat = (prefs: DownloadPrefs = loadPrefs(), nav?: { connection?: Connection }): Format =>
	prefs[networkType(nav ?? (globalThis.navigator as never))];

/** "23.4 MB", "1.20 GB". */
export function formatBytes(n: number): string {
	if (!Number.isFinite(n) || n <= 0) return '0 MB';
	if (n >= 1024 ** 3) return `${(n / 1024 ** 3).toFixed(2)} GB`;
	return `${(n / 1024 ** 2).toFixed(1)} MB`;
}

export interface StorageInfo {
	usage?: number;
	quota?: number;
	persisted?: boolean;
}

type StorageManagerLike = Partial<Pick<StorageManager, 'estimate' | 'persist' | 'persisted'>>;
const storageManager = (): StorageManagerLike | undefined => globalThis.navigator?.storage;

/** How much this site stores and may store, and whether the browser promised to keep it. */
export async function storageInfo(sm: StorageManagerLike | undefined = storageManager()): Promise<StorageInfo> {
	const out: StorageInfo = {};
	try {
		const e = await sm?.estimate?.();
		if (e?.usage !== undefined) out.usage = e.usage;
		if (e?.quota !== undefined) out.quota = e.quota;
	} catch {
		/* not available */
	}
	try {
		const p = await sm?.persisted?.();
		if (p !== undefined) out.persisted = p;
	} catch {
		/* not available */
	}
	return out;
}

/**
 * The browser's storage estimate for the download manager's free-space check, or `{}` when
 * it can't say (`navigator.storage` is withheld from plain-http pages; the check then
 * leaves it to the storage limit and IndexedDB's own quota errors).
 */
export async function storageEstimate(sm: StorageManagerLike | undefined = storageManager()): Promise<{ usage?: number; quota?: number }> {
	try {
		return (await sm?.estimate?.()) ?? {};
	} catch {
		return {};
	}
}

/** Ask the browser not to evict downloads under storage pressure. */
export async function requestPersist(sm: StorageManagerLike | undefined = storageManager()): Promise<boolean> {
	try {
		if (await sm?.persisted?.()) return true;
		return (await sm?.persist?.()) ?? false;
	} catch {
		return false;
	}
}

/** The user whose downloads offline mode shows when the server can't be reached. */
const OWNER_KEY = 'anywhere.offlineUser';

export function rememberOwner(user: string, storage: Set | undefined = local()) {
	try {
		storage?.setItem(OWNER_KEY, user);
	} catch {
		/* offline mode then needs a sign-in first */
	}
}

export function lastOwner(storage: Get | undefined = local()): string | null {
	try {
		const v = storage?.getItem(OWNER_KEY);
		return v && v.length <= 200 ? v : null;
	} catch {
		return null;
	}
}

export function forgetOwner(storage: Pick<Storage, 'removeItem'> | undefined = local()) {
	try {
		storage?.removeItem(OWNER_KEY);
	} catch {
		/* best effort */
	}
}
