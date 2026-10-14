// Changing one's own password (Settings › Account): the checks made before sending, and
// what each refusal says. The gateway's policy is the authority; these only save a trip.
import { ApiError } from '$lib/api/http';
import type { OwnDevice } from '$lib/api/auth';

/** The gateway's minimum length (characters). */
export const MIN_PASSWORD = 10;
/** The gateway's maximum length (bytes). */
export const MAX_PASSWORD = 1024;

export interface PasswordForm {
	current: string;
	next: string;
	confirm: string;
	/** Two-step is on: a 6-digit code is needed. */
	totp: boolean;
	code: string;
}

/** Why the form can't be sent yet, or null if it can. */
export function formProblem(f: PasswordForm): string | null {
	if (!f.current) return 'Enter your current password.';
	if ([...f.next].length < MIN_PASSWORD) return `The new password needs at least ${MIN_PASSWORD} characters.`;
	if (new TextEncoder().encode(f.next).length > MAX_PASSWORD) return 'The new password is too long.';
	if (f.next === f.current) return 'The new password must be different from the current one.';
	if (f.next !== f.confirm) return 'The new passwords don’t match.';
	if (f.totp && !/^\d{6}$/.test(f.code.replace(/\s/g, ''))) return 'Enter the 6-digit code from your authenticator app.';
	return null;
}

/** "in 1 minute", "in 15 minutes", "in 2 hours". */
export function waitText(seconds: number | undefined): string {
	if (!seconds || !Number.isFinite(seconds)) return 'later';
	const mins = Math.max(1, Math.ceil(seconds / 60));
	if (mins < 120) return `in ${mins} minute${mins === 1 ? '' : 's'}`;
	return `in ${Math.ceil(mins / 60)} hours`;
}

/** A server message as a sentence: capital first letter, full stop at the end. */
function sentence(s: string): string {
	const t = s.trim();
	if (!t) return 'The server refused the change.';
	const c = t.charAt(0).toUpperCase() + t.slice(1);
	return /[.!?]$/.test(c) ? c : `${c}.`;
}

/** What went wrong, for the person changing their password. */
export function changeError(err: unknown): { message: string; needCode?: boolean } {
	if (err instanceof ApiError) {
		if (err.status === 403 && err.totpRequired) return { message: 'Your account has two-step verification on: enter the code from your authenticator app.', needCode: true };
		if (err.status === 403) return { message: 'The current password or two-step code isn’t right.' };
		if (err.status === 400) return { message: sentence(err.message) };
		if (err.status === 401) return { message: 'Your session has ended. Sign in again, then change your password.' };
		if (err.status === 413) return { message: 'The new password is too long.' };
		if (err.status === 429) return { message: `Too many attempts. Try again ${waitText(err.retryAfter)}.` };
		if (err.status === 503) return { message: 'The server is busy. Try again in a moment.' };
		return { message: `The server couldn’t change your password (HTTP ${err.status}). Try again later.` };
	}
	if (err instanceof TypeError) return { message: 'Couldn’t reach the server. Check your connection and try again.' };
	return { message: 'Something went wrong. Try again.' };
}

/** "Firefox on Linux · last seen 3 hours ago" for a device in the list. */
export function deviceDetail(d: OwnDevice, nowMs: number = Date.now()): string {
	const parts: string[] = [];
	if (d.client_platform) parts.push(d.client_platform);
	else if (d.client_kind === 'android') parts.push('Android app');
	if (typeof d.last_seen === 'number' && d.last_seen > 0) parts.push(`last seen ${ago(nowMs / 1000 - d.last_seen)}`);
	return parts.join(' · ');
}

function ago(secs: number): string {
	if (secs < 120) return 'just now';
	const m = Math.floor(secs / 60);
	if (m < 120) return `${m} minutes ago`;
	const h = Math.floor(m / 60);
	if (h < 48) return `${h} hours ago`;
	return `${Math.floor(h / 24)} days ago`;
}
