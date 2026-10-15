// Signing out of this device (Settings › Account): what is stopped and forgotten, and that a
// shared device's downloads are never left behind by a sign-out that went ahead anyway.
import { signOutHere, type SignOutParts } from '$lib/account/sign-out';

function parts(over: Partial<{ removeAll: () => Promise<void>; setUser: () => Promise<void> }> = {}) {
	const order: string[] = [];
	const p = {
		device: { stop: vi.fn(() => void order.push('device.stop')) },
		offlinePlayer: { stop: vi.fn(() => void order.push('player.stop')) },
		downloads: {
			removeAll: vi.fn(over.removeAll ?? (async () => void order.push('removeAll'))),
			setUser: vi.fn(over.setUser ?? (async () => void order.push('downloads.setUser'))),
		},
		plays: { setUser: vi.fn(async () => void order.push('plays.setUser')) },
		forgetOwner: vi.fn(() => void order.push('forgetOwner')),
		session: { signOut: vi.fn(async () => void order.push('signOut')) },
	};
	return { p: p as SignOutParts & typeof p, order };
}

test('keeping downloads: stop, forget the owner, clear the user, then sign out', async () => {
	const { p, order } = parts();
	await signOutHere(p, false);
	expect(order).toEqual(['device.stop', 'player.stop', 'forgetOwner', 'downloads.setUser', 'plays.setUser', 'signOut']);
	expect(p.downloads.removeAll).not.toHaveBeenCalled();
});

test('removing downloads removes them before signing out', async () => {
	const { p, order } = parts();
	await signOutHere(p, true);
	expect(order.indexOf('removeAll')).toBeGreaterThan(-1);
	expect(order.indexOf('removeAll')).toBeLessThan(order.indexOf('signOut'));
});

test('if the downloads can’t be removed, it stays signed in (and rejects, to say so)', async () => {
	const { p } = parts({ removeAll: () => Promise.reject(new Error('IndexedDB broke')) });
	await expect(signOutHere(p, true)).rejects.toThrow('IndexedDB broke');
	expect(p.session.signOut).not.toHaveBeenCalled();
});

test('clearing the offline user failing never stops the sign-out', async () => {
	const { p } = parts({ setUser: () => Promise.reject(new Error('IndexedDB broke')) });
	await signOutHere(p, false);
	expect(p.session.signOut).toHaveBeenCalledTimes(1);
});
