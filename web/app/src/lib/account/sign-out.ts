// Signing out of this device (Settings › Account): stop what plays, remove the downloads if
// asked, forget whose they were, then end the session.

export interface SignOutParts {
	device: { stop: () => void };
	offlinePlayer: { stop: () => void };
	downloads: { removeAll: () => Promise<void>; setUser: (u: null) => Promise<unknown> };
	plays: { setUser: (u: null) => Promise<unknown> };
	forgetOwner: () => void;
	session: { signOut: () => Promise<void> };
}

/**
 * Sign out. When the downloads are to go (a shared device) and they can't be removed, it
 * stays signed in and rejects, so the person can say what to do rather than leave them behind.
 * Clearing the offline user is best effort: it never stops the sign-out.
 */
export async function signOutHere(p: SignOutParts, removeDownloads: boolean): Promise<void> {
	p.device.stop();
	p.offlinePlayer.stop();
	if (removeDownloads) await p.downloads.removeAll();
	// Offline mode no longer offers this account's downloads without signing in.
	p.forgetOwner();
	await p.downloads.setUser(null).catch(() => {});
	await p.plays.setUser(null).catch(() => {});
	await p.session.signOut();
}
