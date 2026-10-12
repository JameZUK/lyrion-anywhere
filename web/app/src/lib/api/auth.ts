// Sign-in for the browser (spec §9). The refresh token lives only in an HttpOnly cookie the
// page can't read; the 15-minute access token is kept in memory only.
import { bearer, callJson } from './http';

export interface User {
	username: string;
	role: 'admin' | 'listener';
}
export interface Device {
	id: number;
	name: string;
	mac?: string;
}
export interface Session {
	access_token: string;
	expires_in: number;
	user: User;
	device: Device;
}
export interface Account {
	username: string;
	role: 'admin' | 'listener';
	totp_enabled: boolean;
}
export interface TotpSetup {
	secret: string;
	uri: string;
	qr_svg: string;
}

const DEVICE_KEY = 'anywhere.device';

/** The device id this browser used last time (not a secret), if any. */
export function rememberedDevice(storage: Pick<Storage, 'getItem'> | undefined = globalThis.localStorage): number | undefined {
	try {
		const v = Number(storage?.getItem(DEVICE_KEY));
		return Number.isSafeInteger(v) && v > 0 ? v : undefined;
	} catch {
		return undefined; // storage blocked (private mode etc.)
	}
}

function rememberDevice(id: number, storage: Pick<Storage, 'setItem'> | undefined = globalThis.localStorage) {
	try {
		storage?.setItem(DEVICE_KEY, String(id));
	} catch {
		/* not essential */
	}
}

/** Touch points of this device's screen (iPads report themselves as Macs, but have touch). */
function touchPoints(): number {
	try {
		return Number(globalThis.navigator?.maxTouchPoints) || 0;
	} catch {
		return 0;
	}
}

/**
 * A readable default name for this browser's player, e.g. "Firefox on Linux" or
 * "Safari on iPadOS". Also sent as the client's platform at sign-in.
 */
export function defaultDeviceName(ua: string = globalThis.navigator?.userAgent ?? '', touch: number = touchPoints()): string {
	const browser = /Firefox\/|FxiOS\//.test(ua)
		? 'Firefox'
		: /Edg\//.test(ua)
			? 'Edge'
			: /Chrome\/|CriOS\//.test(ua)
				? 'Chrome'
				: /Safari\//.test(ua)
					? 'Safari'
					: 'Browser';
	const os = /Android/.test(ua)
		? 'Android'
		: /iPad/.test(ua) || (/Macintosh/.test(ua) && touch > 1)
			? 'iPadOS'
			: /iPhone|iPod/.test(ua)
				? 'iOS'
				: /Mac OS X/.test(ua)
					? 'macOS'
					: /Windows/.test(ua)
						? 'Windows'
						: /Linux/.test(ua)
							? 'Linux'
							: 'unknown';
	return `${browser} on ${os}`;
}

/** How this web app runs: installed (standalone display, a PWA) or in a browser tab. */
export type ClientKind = 'pwa' | 'browser';

export function clientKind(
	matches: (query: string) => boolean = (q) => typeof globalThis.matchMedia === 'function' && globalThis.matchMedia(q).matches,
	iosStandalone: unknown = (globalThis.navigator as { standalone?: unknown } | undefined)?.standalone,
): ClientKind {
	try {
		if (iosStandalone === true) return 'pwa';
		for (const mode of ['standalone', 'fullscreen', 'minimal-ui', 'window-controls-overlay']) {
			if (matches(`(display-mode: ${mode})`)) return 'pwa';
		}
	} catch {
		/* no matchMedia: a tab */
	}
	return 'browser';
}

/** What the gateway is told about this client at sign-in and renewal (shown to admins). */
export function clientInfo(): { client_kind: ClientKind; client_platform: string } {
	return { client_kind: clientKind(), client_platform: defaultDeviceName().slice(0, 48) };
}

export async function login(username: string, password: string, totp?: string, deviceName = defaultDeviceName()): Promise<Session> {
	const session = await callJson<Session>('/v1/auth/login', {
		method: 'POST',
		headers: { 'Content-Type': 'application/json' },
		body: JSON.stringify({
			username,
			password,
			...(totp ? { totp } : {}),
			device_name: deviceName,
			device_id: rememberedDevice(),
			...clientInfo(),
		}),
	});
	rememberDevice(session.device.id);
	return session;
}

/** The Web Locks lock that refreshes run under, across all of this site's tabs. */
export const REFRESH_LOCK = 'anywhere-refresh';

/** The part of `navigator.locks` used here. */
export interface LockManagerLike {
	request<T>(name: string, callback: () => Promise<T>): Promise<T>;
}

function browserLocks(): LockManagerLike | undefined {
	try {
		return (globalThis.navigator as { locks?: LockManagerLike } | undefined)?.locks;
	} catch {
		return undefined;
	}
}

/**
 * Run `fn` holding the cross-tab refresh lock, so two tabs (or a tab and the installed app)
 * never present the same refresh cookie at once: the second waits, then sends the cookie
 * the first one received. Without Web Locks it simply runs; the gateway tolerates
 * concurrent refreshes within a short grace window.
 */
export function withRefreshLock<T>(fn: () => Promise<T>, locks: LockManagerLike | undefined = browserLocks()): Promise<T> {
	if (!locks || typeof locks.request !== 'function') return fn();
	return locks.request(REFRESH_LOCK, fn);
}

/** Exchange the refresh cookie for a new access token (and a rotated cookie). */
export async function refresh(locks: LockManagerLike | undefined = browserLocks()): Promise<Session> {
	return withRefreshLock(async () => {
		const session = await callJson<Session>('/v1/auth/refresh', {
			method: 'POST',
			headers: { 'Content-Type': 'application/json' },
			body: JSON.stringify(clientInfo()),
		});
		rememberDevice(session.device.id);
		return session;
	}, locks);
}

export async function logout(token: string): Promise<void> {
	await callJson('/v1/auth/logout', { method: 'POST', headers: bearer(token) });
}

/** Device names: 1 to 64 characters (the gateway cleans them further). Trimmed, or null. */
export function validDeviceName(v: string): string | null {
	const s = v.trim();
	const n = [...s].length;
	return n >= 1 && n <= 64 && !/[\u0000-\u001f\u007f-\u009f]/.test(s) ? s : null;
}

/** Rename this device (its LMS player follows). Returns the name the gateway stored. */
export const renameDevice = (token: string, name: string) =>
	callJson<{ id: number; name: string }>('/v1/devices/self', {
		method: 'PATCH',
		headers: { ...bearer(token), 'Content-Type': 'application/json' },
		body: JSON.stringify({ name }),
	});

export const account = (token: string) => callJson<Account>('/v1/account', { headers: bearer(token) });

export interface PairingCode {
	/** One-time code: 10 letters and digits. */
	code: string;
	/** Seconds it stays valid (600). */
	expires_in: number;
}

/**
 * A one-time code that signs another device in as this user, e.g. the Android app scanning
 * it as a QR code. The gateway asks for the account password, and allows a few an hour.
 */
export const createPairingCode = (token: string, password: string) =>
	callJson<PairingCode>('/v1/devices/pair', {
		method: 'POST',
		headers: { ...bearer(token), 'Content-Type': 'application/json' },
		body: JSON.stringify({ password }),
	});

/** Start two-step setup. The gateway asks for the account password first. */
export const totpSetup = (token: string, password: string) =>
	callJson<TotpSetup>('/v1/account/totp/setup', {
		method: 'POST',
		headers: { ...bearer(token), 'Content-Type': 'application/json' },
		body: JSON.stringify({ password }),
	});

export async function totpConfirm(token: string, code: string): Promise<void> {
	await callJson('/v1/account/totp/confirm', {
		method: 'POST',
		headers: { ...bearer(token), 'Content-Type': 'application/json' },
		body: JSON.stringify({ code }),
	});
}

export async function totpDisable(token: string, password: string, code: string): Promise<void> {
	await callJson('/v1/account/totp/disable', {
		method: 'POST',
		headers: { ...bearer(token), 'Content-Type': 'application/json' },
		body: JSON.stringify({ password, code }),
	});
}

/** Group a base32 key in fours for reading aloud or typing. */
export const groupKey = (secret: string) => secret.replace(/(.{4})(?=.)/g, '$1 ');
