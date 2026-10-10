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

/** A readable default name for this browser's player, e.g. "Firefox on Linux". */
export function defaultDeviceName(ua: string = globalThis.navigator?.userAgent ?? ''): string {
	const browser = /Firefox\//.test(ua)
		? 'Firefox'
		: /Edg\//.test(ua)
			? 'Edge'
			: /Chrome\//.test(ua)
				? 'Chrome'
				: /Safari\//.test(ua)
					? 'Safari'
					: 'Browser';
	const os = /Android/.test(ua)
		? 'Android'
		: /iPhone|iPad|iPod/.test(ua)
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
		const session = await callJson<Session>('/v1/auth/refresh', { method: 'POST' });
		rememberDevice(session.device.id);
		return session;
	}, locks);
}

export async function logout(token: string): Promise<void> {
	await callJson('/v1/auth/logout', { method: 'POST', headers: bearer(token) });
}

export const account = (token: string) => callJson<Account>('/v1/account', { headers: bearer(token) });

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
