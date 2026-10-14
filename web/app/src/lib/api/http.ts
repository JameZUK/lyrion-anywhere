// Low-level calls to the gateway's /v1 API (docs/PROTOCOL.md).
//
// Every request carries X-Anywhere-Client: a cross-site page can't send it without a CORS
// preflight the gateway never approves, which is the CSRF defence for the refresh cookie.

export const CLIENT_HEADER = { 'X-Anywhere-Client': '1' } as const;

export class ApiError extends Error {
	constructor(
		readonly status: number,
		message: string,
		/** The password was right: ask for a two-step code and sign in again. */
		readonly totpRequired = false,
		/** Seconds to wait before trying again (the `Retry-After` header), if the server said. */
		readonly retryAfter?: number,
	) {
		super(message);
		this.name = 'ApiError';
	}
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
		const msg = typeof body.error === 'string' ? body.error : `HTTP ${resp.status}`;
		throw new ApiError(resp.status, msg, body.totp_required === true, retryAfterOf(resp.headers?.get?.('retry-after') ?? null));
	}
	return body as T;
}

/** A `Retry-After` in whole seconds (1 to a day); dates and nonsense give undefined. */
export function retryAfterOf(v: string | null): number | undefined {
	if (v === null || !/^\s*\d{1,9}\s*$/.test(v)) return undefined;
	return Math.min(Math.max(Number(v), 1), 86_400);
}

export const bearer = (token: string) => ({ Authorization: `Bearer ${token}` });
