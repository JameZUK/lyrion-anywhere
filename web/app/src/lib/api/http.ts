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
		/** Seconds to wait before trying again (the `Retry-After` header), if the server said. */
		readonly retryAfter?: number,
		/** The gateway's whole answer, for refusals that say more than a message. */
		readonly body: Record<string, unknown> = {},
	) {
		super(message);
		this.name = 'ApiError';
	}

	/** The gateway refused at once because LMS isn't answering (503 with `busy: true`). */
	get busy(): boolean {
		return this.status === 503 && this.body.busy === true;
	}
}

/** Is this the gateway saying the music server is busy (rather than a real failure)? */
export function isBusy(e: unknown): e is ApiError {
	return e instanceof ApiError && e.status === 503 && e.busy;
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
		throw new ApiError(resp.status, msg, body.totp_required === true, retryAfterOf(resp.headers?.get?.('retry-after') ?? null), body);
	}
	return body as T;
}

/** A `Retry-After` in whole seconds (1 to a day); dates and nonsense give undefined. */
export function retryAfterOf(v: string | null): number | undefined {
	if (v === null || !/^\s*\d{1,9}\s*$/.test(v)) return undefined;
	return Math.min(Math.max(Number(v), 1), 86_400);
}

export const bearer = (token: string) => ({ Authorization: `Bearer ${token}` });
