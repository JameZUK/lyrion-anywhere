// "Your devices" (Settings › Account): the list as the gateway sent it, checked; how each
// device reads ("Pixel 8 · Last used 3 hours ago"); and what each refusal says.
import { ApiError } from '#lib/api/http';
import type { MyDevice } from '#lib/api/auth';
import { waitText } from './change-password';
import { ago } from './ago';

export { ago };

/** Most devices read from an answer (the gateway allows 20 a user). */
export const MAX_DEVICES = 64;

const KINDS = new Set(['android', 'pwa', 'browser']);

/** Visible text only: no control or format characters, at most `max` characters. */
function text(v: unknown, max: number): string | null {
	if (typeof v !== 'string') return null;
	const t = [...v.replace(/[\p{Cc}\p{Cf}]/gu, '')].slice(0, max).join('').trim();
	return t || null;
}

const seconds = (v: unknown): number | null => (typeof v === 'number' && Number.isSafeInteger(v) && v > 0 ? v : null);

/**
 * The gateway's list, checked: entries without a positive whole id or a name are dropped,
 * so are repeated ids; at most one device is taken as this one.
 */
export function cleanDevices(raw: unknown): MyDevice[] {
	if (!Array.isArray(raw)) throw new ApiError(0, 'malformed answer');
	const out: MyDevice[] = [];
	const ids = new Set<number>();
	let current = false;
	for (const d of raw.slice(0, MAX_DEVICES)) {
		if (!d || typeof d !== 'object') continue;
		const r = d as Record<string, unknown>;
		const id = r.id;
		const name = text(r.name, 64);
		if (typeof id !== 'number' || !Number.isSafeInteger(id) || id <= 0 || ids.has(id) || !name) continue;
		ids.add(id);
		const isCurrent: boolean = r.current === true && !current;
		current ||= isCurrent;
		out.push({
			id,
			name,
			client_kind: typeof r.client_kind === 'string' && KINDS.has(r.client_kind) ? r.client_kind : null,
			client_platform: text(r.client_platform, 48),
			created_at: seconds(r.created_at),
			last_seen: seconds(r.last_seen),
			current: isCurrent,
		});
	}
	return out;
}

/** What kind of device, when it didn't name its platform. */
function kindText(kind: string | null): string | null {
	if (kind === 'android') return 'Android app';
	if (kind === 'pwa') return 'Installed web app';
	if (kind === 'browser') return 'Web browser';
	return null;
}

/** "Pixel 8 · Last used 3 hours ago" (this device: "Pixel 8 · In use now"). */
export function deviceLine(d: MyDevice, nowMs: number = Date.now()): string {
	const parts: string[] = [];
	const what = d.client_platform ?? kindText(d.client_kind);
	if (what) parts.push(what);
	if (d.current) parts.push('In use now');
	else if (d.last_seen) parts.push(`Last used ${ago(Math.max(0, nowMs / 1000 - d.last_seen))}`);
	else parts.push('Not used yet');
	return parts.join(' · ');
}

/** What went wrong signing a device out (or loading the list), as a sentence. */
export function devicesError(err: unknown): string {
	if (err instanceof ApiError) {
		if (err.status === 401) return 'Your session has ended. Sign in again.';
		if (err.status === 404) return 'That device was already signed out.';
		if (err.status === 409) return 'This is the device you’re using. To sign it out, use Sign out.';
		if (err.status === 429) return `Too many requests. Try again ${waitText(err.retryAfter)}.`;
		if (err.status === 503) return 'The server is busy. Try again in a moment.';
		if (err.status === 0) return 'The server sent something unexpected. Try again later.';
		return `The server couldn’t do that (HTTP ${err.status}). Try again later.`;
	}
	if (err instanceof TypeError) return 'Couldn’t reach the server. Check your connection and try again.';
	return 'Something went wrong. Try again.';
}
