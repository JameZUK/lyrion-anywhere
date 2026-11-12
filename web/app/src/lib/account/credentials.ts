// What a refused password or two-step code says in Settings › Account (two-step on and off,
// Add another device). The gateway answers wrong credentials there with 403, as it does for a
// password change, never with 401: that is the access token, renewed without sending what
// the person typed again (see SessionManager.authedOnce).
import { ApiError } from '#lib/api/http';
import { RenewedFirst } from '#lib/api/session';
import { waitText } from './change-password';

/**
 * The message for a failed credentials call. `wrong`: what a wrong password (or code) says;
 * `limited`: what too many attempts say (by default, when to try again).
 */
export function credentialsError(err: unknown, wrong: string, limited?: string): string {
	if (err instanceof RenewedFirst) return err.message;
	if (err instanceof ApiError) {
		if (err.status === 403) return wrong;
		if (err.status === 429) return limited ?? `Too many attempts. Try again ${waitText(err.retryAfter)}.`;
		if (err.status === 401) return 'Your sign-in has ended. Sign in again, then try again.';
		if (err.status === 503) return 'The server is busy. Try again in a moment.';
		return err.message;
	}
	if (err instanceof TypeError) return 'Couldn’t reach the server. Check your connection and try again.';
	return err instanceof Error && err.message ? err.message : 'Something went wrong. Try again.';
}
