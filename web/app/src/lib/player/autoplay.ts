// Does this browser let the media element start without a tap?
//
// Safari (iOS, iPadOS, and a reopened home-screen app) refuses play() without a user gesture
// with NotAllowedError. Trying anyway only shows "tap to resume" after a failed start, and the
// failed start can leave the element half-started. So a refusal is remembered, and the next
// automatic start asks for the tap first. Firefox says so up front (getAutoplayPolicy); a
// gesture already in hand (navigator.userActivation) always lets it try.

const KEY = 'anywhere.playback.gesture';

interface AutoplayNavigator {
	userActivation?: { isActive?: boolean };
	getAutoplayPolicy?: (type: string) => string;
}

/** Would an automatic start's play() be refused for want of a gesture? */
export function needsGesture(nav: AutoplayNavigator | undefined = globalThis.navigator as AutoplayNavigator | undefined): boolean {
	if (nav?.userActivation?.isActive === true) return false;
	try {
		const policy = nav?.getAutoplayPolicy?.('mediaelement');
		if (policy === 'allowed') return false;
		// Muted only is no use to a player.
		if (policy === 'allowed-muted' || policy === 'disallowed') return true;
	} catch {
		/* not supported here */
	}
	try {
		return localStorage.getItem(KEY) === 'needed';
	} catch {
		return false;
	}
}

/** Remember how this browser answered an automatic start (refused: `true`). */
export function rememberAutoplay(refused: boolean) {
	try {
		if (refused) localStorage.setItem(KEY, 'needed');
		else localStorage.removeItem(KEY);
	} catch {
		/* storage unavailable: ask again next time */
	}
}
