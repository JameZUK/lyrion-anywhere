// Stores holding one user's data must not show it to the next person who signs in on the
// same tab: they are reset whenever the signed-in user changes (signing out included).
import type { Session } from '#lib/api/auth';

interface Subscribable {
	subscribe(fn: (s: Session | null, status: string) => void): () => void;
}

/**
 * Signed in as someone other than the user last seen signed in, with no sign-out seen in
 * between. A sign-in that ends and another that starts at once (the shared cookie came back
 * as someone else: see SessionManager) can reach an effect as one change; this is it.
 */
export function switchedUser(last: string | null, status: string, user: string | null): boolean {
	return status === 'signed-in' && !!user && !!last && user !== last;
}

/** Call `reset` each time the signed-in user changes (not on the first report, nor on renewals). */
export function resetOnUserChange(session: Subscribable, reset: () => void): () => void {
	let user: string | null | undefined;
	return session.subscribe((s) => {
		const now = s?.user.username ?? null;
		if (user !== undefined && now !== user) reset();
		user = now;
	});
}
