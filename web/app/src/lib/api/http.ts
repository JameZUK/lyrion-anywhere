// Low-level calls to the gateway's /v1 API (docs/PROTOCOL.md).
//
// Every request carries X-Anywhere-Client: a cross-site page can't send it without a CORS
// preflight the gateway never approves, which is the CSRF defence for the refresh cookie.

export const CLIENT_HEADER = { 'X-Anywhere-Client': '1' } as const;

/** What the user is told when the music server is busy (and a request didn't happen). */
export const BUSY_MESSAGE = 'The music server is busy; try again in a moment.';

export class ApiError extends Error {
	constructor(
		readonly status: number,
		message: string,
		/** The password was right: ask for a two-step code and sign in again. */
		readonly totpRequired = false,
		/** The gateway refused at once because LMS isn't answering (503 with `busy: true`). */
		readonly busy = false,
		/** Seconds to wait before trying again (the Retry-After header), or null. */
		readonly retryAfter: number | null = null,
	) {
		super(message);
		this.name = 'ApiError';
	}
}

/** Is this the gateway saying the music server is busy (rather than a real failure)? */
export function isBusy(e: unknown): e is ApiError {
	return e instanceof ApiError && e.status === 503 && e.busy;
}

/**
 * A Retry-After header in whole seconds (0 to 300), or null if absent or not plain seconds.
 * HTTP dates and anything else are ignored: the caller's own backoff applies.
 */
export function retryAfterOf(header: string | null | undefined): number | null {
	if (typeof header !== 'string' || !/^\s*\d{1,6}\s*$/.test(header)) return null;
	return Math.min(Number(header.trim()), 300);
}

/** fetch() that returns parsed JSON (or null for 204) and throws ApiError on failure. */
export async function callJson<T>(path: string, init: RequestInit = {}): Promise<T> {
	const resp = await fetch(path, {
		credentials: 'same-origin',
		cache: 'no-store',
		...init,
		headers: { ...CLIENT_HEADER, ...(init.headers as Record<string, string> | undefined) },
	});
	if (resp.status === 204) return null as T;
	const body = (await resp.json().catch(() => ({}))) as Record<string, unknown>;
	if (!resp.ok) {
		const busy = resp.status === 503 && body.busy === true;
		const msg = busy ? BUSY_MESSAGE : typeof body.error === 'string' ? body.error : `HTTP ${resp.status}`;
		const retry = retryAfterOf(resp.headers?.get?.('Retry-After'));
		throw new ApiError(resp.status, msg, body.totp_required === true, busy, retry);
	}
	return body as T;
}

export const bearer = (token: string) => ({ Authorization: `Bearer ${token}` });
