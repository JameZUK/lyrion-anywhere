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

/**
 * The longest an API call may take, its answer and body together. Past it the call counts as
 * the server being out of reach (a connection that died without closing, say). Streams and
 * downloads don't come through here: they have their own handling of a stalled transfer.
 */
export const API_TIMEOUT_MS = 15_000;

/** An API call that took too long. A TypeError, as fetch's own network errors are. */
export class RequestTimeout extends TypeError {
	constructor() {
		super('The music server took too long to answer.');
		this.name = 'RequestTimeout';
	}
}

/**
 * fetch() that returns parsed JSON (or null for 204) and throws ApiError on failure. Gives up
 * after `timeoutMs` with {@link RequestTimeout}; a `signal` of the caller's still aborts it.
 */
export async function callJson<T>(path: string, init: RequestInit = {}, timeoutMs: number = API_TIMEOUT_MS): Promise<T> {
	const ctl = new AbortController();
	const outer = init.signal;
	let timedOut = false;
	const timer = setTimeout(() => {
		if (ctl.signal.aborted) return; // the caller's own abort came first
		timedOut = true;
		ctl.abort();
	}, timeoutMs);
	const follow = () => ctl.abort(outer?.reason);
	if (outer?.aborted) follow();
	else outer?.addEventListener('abort', follow, { once: true });
	try {
		const resp = await fetch(path, {
			credentials: 'same-origin',
			cache: 'no-store',
			...init,
			signal: ctl.signal,
			headers: { ...CLIENT_HEADER, ...(init.headers as Record<string, string> | undefined) },
		});
		if (resp.status === 204) return null as T;
		// A body cut short by the limit (or an abort) is not an empty answer.
		const text = await resp.text().catch((e) => {
			if (ctl.signal.aborted) throw e;
			return '';
		});
		let parsed: unknown = {};
		let malformed = false;
		if (text.trim()) {
			try {
				parsed = JSON.parse(text);
			} catch {
				malformed = true;
			}
		}
		// A success that isn't JSON came from something in the way (a proxy's fallback page,
		// say), not the gateway: never read it as an empty answer.
		if (resp.ok && malformed) throw new ApiError(502, 'the server sent something that isn’t the gateway’s answer');
		const body = (parsed && typeof parsed === 'object' ? parsed : {}) as Record<string, unknown>;
		if (!resp.ok) {
			const busy = resp.status === 503 && body.busy === true;
			const msg = busy ? BUSY_MESSAGE : typeof body.error === 'string' ? body.error : `HTTP ${resp.status}`;
			throw new ApiError(resp.status, msg, body.totp_required === true, retryAfterOf(resp.headers?.get?.('retry-after') ?? null), body);
		}
		return parsed as T;
	} catch (e) {
		if (timedOut) throw new RequestTimeout();
		throw e;
	} finally {
		clearTimeout(timer);
		outer?.removeEventListener('abort', follow);
	}
}

/** A `Retry-After` in whole seconds (1 to a day); dates and nonsense give undefined. */
export function retryAfterOf(v: string | null): number | undefined {
	if (v === null || !/^\s*\d{1,9}\s*$/.test(v)) return undefined;
	return Math.min(Math.max(Number(v), 1), 86_400);
}

export const bearer = (token: string) => ({ Authorization: `Bearer ${token}` });
