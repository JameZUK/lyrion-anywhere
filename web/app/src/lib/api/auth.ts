// Sign-in for the browser (spec §9). The refresh token lives only in an HttpOnly cookie the
// page can't read; the 15-minute access token is kept in memory only.
import { ApiError, bearer, callJson } from './http';

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

/** The longest a refresh request may take before it is abandoned (and tried again later). */
export const REFRESH_TIMEOUT_MS = 15_000;
/** The longest one tab may hold the refresh lock, whatever happens inside. */
export const LOCK_HOLD_MS = 20_000;
/** The longest a tab waits for another tab's refresh before giving up for now. */
export const LOCK_WAIT_MS = 45_000;

/**
 * A refresh that took too long (the request, or the wait for another tab's). Not a refusal:
 * the session keeps its sign-in and tries again shortly.
 */
export class RefreshTimeout extends Error {
	constructor(what: string) {
		super(what);
		this.name = 'RefreshTimeout';
	}
}

export interface LockTimes {
	holdMs?: number;
	waitMs?: number;
}

/**
 * Run `fn` holding the cross-tab refresh lock, so two tabs (or a tab and the installed app)
 * never present the same refresh cookie at once: the second waits, then sends the cookie
 * the first one received. Without Web Locks it simply runs; the gateway tolerates
 * concurrent refreshes within a short grace window.
 *
 * Both sides are bounded: a holder lets go after `holdMs` even if `fn` hangs, and a waiter
 * gives up after `waitMs` (and then never runs `fn`). A hung refresh used to hold the lock for
 * ever, blocking every tab, the events socket and the player worker.
 */
export function withRefreshLock<T>(fn: () => Promise<T>, locks: LockManagerLike | undefined = browserLocks(), times: LockTimes = {}): Promise<T> {
	const holdMs = times.holdMs ?? LOCK_HOLD_MS;
	const waitMs = times.waitMs ?? LOCK_WAIT_MS;
	if (!locks || typeof locks.request !== 'function') return fn();
	let granted = false;
	let abandoned = false;
	return new Promise<T>((resolve, reject) => {
		const waiting = setTimeout(() => {
			if (granted) return;
			abandoned = true;
			reject(new RefreshTimeout('another tab’s refresh is taking too long'));
		}, waitMs);
		locks
			.request(REFRESH_LOCK, () => {
				granted = true;
				clearTimeout(waiting);
				if (abandoned) return Promise.resolve(undefined as T); // gave up waiting: do nothing
				return new Promise<T>((res, rej) => {
					const holding = setTimeout(() => rej(new RefreshTimeout('the refresh is taking too long')), holdMs);
					fn().then(
						(v) => (clearTimeout(holding), res(v)),
						(e) => (clearTimeout(holding), rej(e)),
					);
				});
			})
			.then(
				(v) => !abandoned && resolve(v),
				(e) => !abandoned && reject(e),
			);
	});
}

/** Why a refresh was asked for. Sent to the gateway's log only, to trace a reused token. */
export type RefreshReason = 'load' | 'timer' | '401' | 'events' | 'player' | 'retry' | 'signout';

/** A random id for this page load, so the gateway's log can tell two copies of the app apart. */
export const PAGE_ID = Math.random().toString(36).slice(2, 10) || 'page';

/** Exchange the refresh cookie for a new access token (and a rotated cookie). */
export async function refresh(reason: RefreshReason = 'load', locks: LockManagerLike | undefined = browserLocks(), times: LockTimes & { requestMs?: number } = {}): Promise<Session> {
	return withRefreshLock(
		async () => {
			// A request that never answers is abandoned, so the lock is let go for the next tab.
			const ctl = new AbortController();
			const timer = setTimeout(() => ctl.abort(), times.requestMs ?? REFRESH_TIMEOUT_MS);
			try {
				const session = await callJson<Session>('/v1/auth/refresh', {
					method: 'POST',
					headers: { 'Content-Type': 'application/json', 'X-Anywhere-Refresh-Reason': reason, 'X-Anywhere-Page': PAGE_ID },
					body: JSON.stringify(clientInfo()),
					signal: ctl.signal,
				});
				rememberDevice(session.device.id);
				return session;
			} catch (e) {
				if (ctl.signal.aborted) throw new RefreshTimeout('the refresh request did not answer');
				throw e;
			} finally {
				clearTimeout(timer);
			}
		},
		locks,
		times,
	);
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

/** Optional features the gateway offers (`GET /v1/me`); missing ones are off. */
export interface Features {
	change_password?: boolean;
	/** "Your devices": `GET /v1/devices` and signing devices out. */
	devices?: boolean;
}

/** What this gateway offers. An older gateway that doesn't say gives no features. */
export async function features(token: string): Promise<Features> {
	const me = await callJson<{ features?: unknown }>('/v1/me', { headers: bearer(token) });
	const f = me?.features;
	if (!f || typeof f !== 'object') return {};
	const r = f as Record<string, unknown>;
	return { change_password: r.change_password === true, devices: r.devices === true };
}

/** One of the user's own devices, as the gateway lists them. */
export interface OwnDevice {
	id: number;
	name: string;
	client_kind?: string | null;
	client_platform?: string | null;
	/** Unix seconds. */
	last_seen?: number | null;
}

export interface PasswordChanged {
	/** How many other devices were signed out with the change. */
	signed_out: number;
	/** The user's other devices, still signed in. */
	other_devices: OwnDevice[];
}

export interface PasswordChange {
	current: string;
	new: string;
	/** The two-step code, when two-step is on. */
	code?: string;
	/** Also sign out this user's other devices. */
	signOutOthers?: boolean;
}

/**
 * Change this user's own password. Wrong credentials come back as 403 (not 401), so the
 * session doesn't renew its token and send the same guess twice.
 */
export const changePassword = (token: string, c: PasswordChange) =>
	callJson<PasswordChanged>('/v1/account/password', {
		method: 'POST',
		headers: { ...bearer(token), 'Content-Type': 'application/json' },
		body: JSON.stringify({
			current: c.current,
			new: c.new,
			...(c.code ? { code: c.code } : {}),
			...(c.signOutOthers ? { sign_out_others: true } : {}),
		}),
	});

/** One of the user's signed-in devices, for "Your devices" (`GET /v1/devices`). */
export interface MyDevice {
	id: number;
	name: string;
	client_kind: string | null;
	client_platform: string | null;
	/** Unix seconds. */
	created_at: number | null;
	/** Unix seconds, to the minute. */
	last_seen: number | null;
	/** The device asking: this one. */
	current: boolean;
}

/** The raw device list (checked by `cleanDevices` in `$lib/account/devices`). */
export async function myDevices(token: string): Promise<unknown> {
	const r = await callJson<{ devices?: unknown }>('/v1/devices', { headers: bearer(token) });
	return r?.devices;
}

/** Sign out another of this user's devices. 404: it had gone already (or never was theirs). */
export async function signOutDevice(token: string, id: number): Promise<void> {
	if (!Number.isSafeInteger(id) || id <= 0) throw new ApiError(404, 'no such device');
	await callJson(`/v1/devices/${id}`, { method: 'DELETE', headers: bearer(token) });
}

/** Sign out all this user's devices but this one; how many were signed out. */
export async function signOutOtherDevices(token: string): Promise<number> {
	const r = await callJson<{ signed_out?: unknown }>('/v1/devices/sign-out-others', { method: 'POST', headers: bearer(token) });
	const n = Number(r?.signed_out);
	return Number.isSafeInteger(n) && n >= 0 ? n : 0;
}

/** Group a base32 key in fours for reading aloud or typing. */
export const groupKey = (secret: string) => secret.replace(/(.{4})(?=.)/g, '$1 ');
