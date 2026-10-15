// What a failed sign-in says. Each gateway refusal reads as what it is; only a request that
// never reached the server blames the connection (a proxy that drops the Origin header, a busy
// music server and a server fault each look different from a dead network).
import { ApiError } from '$lib/api/http';

export const CONNECTION_MESSAGE = 'Could not reach the server. Check your connection.';

/** The message for a sign-in that failed with `err` (`withCode`: a two-step code was sent). */
export function loginError(err: unknown, withCode = false): string {
	if (err instanceof ApiError) {
		if (err.totpRequired) return 'Enter the code from your authenticator app.';
		if (err.status === 401) return withCode ? 'That code or password is not right.' : 'Username or password is not right.';
		if (err.status === 429) return 'Too many attempts. Wait a while and try again.';
		if (err.status === 403) return 'The server refused this sign-in. If the app is behind a proxy, check that it passes the Origin header.';
		if (err.status === 503) return 'The music server is busy. Try again shortly.';
		if (err.status >= 500) return 'The server had a problem. Try again shortly.';
		if (err.status === 400) return 'The server didn’t accept that sign-in. Check the username and password.';
		return `The server refused this sign-in (HTTP ${err.status}).`;
	}
	if (err instanceof TypeError) return CONNECTION_MESSAGE;
	return 'Something went wrong. Try again.';
}
