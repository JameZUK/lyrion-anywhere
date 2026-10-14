// In-app navigation. The app uses SvelteKit's hash router: assigning `location.hash` by hand
// makes SvelteKit reload the whole page (it treats it like an edit of the address bar), which
// stops this device's player mid-song. Every in-app navigation goes through here instead.
import { goto } from '$app/navigation';

/** The hash link for an in-app route such as `/playlist/3`. */
export function href(route: string): string {
	const r = route.replace(/^#/, '');
	return `#${r.startsWith('/') ? r : `/${r}`}`;
}

/** Go to an in-app route (e.g. `/playlists`) without reloading the page. */
export function go(route: string, opts: { replaceState?: boolean; keepFocus?: boolean; noScroll?: boolean } = {}): Promise<void> {
	return goto(href(route), opts);
}

// --- Hash changes from outside the router -----------------------------------------------
// A route typed into the address bar, or opened from a link elsewhere, changes the hash
// without SvelteKit's router, and SvelteKit then reloads the whole page. That stops this
// device's player, and while the server is out of reach a reload can't sign in again. Such a
// change is routed in the app instead.

/** The key SvelteKit puts in the history entries it makes itself. */
const ROUTER_ENTRY = 'sveltekit:history';

/** The route part of a hash (`#/album/7#top` is `/album/7`). */
const routeOf = (hash: string) => hash.replace(/^#/, '').split('#')[0] || '/';

/**
 * The in-app link to route to for a `popstate` with history `state`, now that the hash is
 * `now` while the app shows `shown`, or null to leave it to SvelteKit: its own history entries
 * (Back and Forward), the same page (an anchor on it), and anything that isn't an app route.
 */
export function outsideRoute(state: unknown, now: string, shown: string): string | null {
	if (routerEntry(state)) return null;
	const route = routeOf(now);
	if (!route.startsWith('/') || route.startsWith('//') || route === routeOf(shown)) return null;
	return href(route);
}

const routerEntry = (state: unknown) => !!state && typeof state === 'object' && ROUTER_ENTRY in state;

/**
 * Route hash changes made outside the router in the app (see above). `shown` is the hash of
 * the page the app shows. Returns a function that stops it.
 *
 * The browser has already added a history entry the router knows nothing about, so the app
 * steps back over it first (to the entry of the page it shows, which the router ignores), and
 * then goes to the route as if a link had been followed: history stays as the router expects.
 */
export function routeOutsideHashChanges(shown: () => string): () => void {
	let pending: string | null = null;
	const onpopstate = (e: PopStateEvent) => {
		if (pending !== null) {
			// Stepped back to the page shown: now go on to the route asked for.
			const to = pending;
			pending = null;
			// After the router has seen this event too (it would cancel a navigation started
			// before it ran).
			if (routerEntry(e.state)) setTimeout(() => void goto(to), 0);
			return;
		}
		const to = outsideRoute(e.state, location.hash, shown());
		if (to === null) return;
		// Before SvelteKit's own listener sees it (capturing listeners on the window run first).
		e.stopImmediatePropagation();
		pending = to;
		history.back();
	};
	addEventListener('popstate', onpopstate, { capture: true });
	return () => removeEventListener('popstate', onpopstate, { capture: true });
}

// --- Back -------------------------------------------------------------------------------
// An installed iPhone web app has no browser Back button, so detail pages have their own. It
// goes back through the app's own history when there is some (so it returns to wherever the
// user came from: search, an artist, Home…), else up to the page's parent.

/** How many in-app pages are behind the current one in this tab's history. */
let depth = 0;

/** What afterNavigate reports, as far as Back needs it. */
export interface NavigationNote {
	type: string;
	delta?: number;
}

/** Note a navigation (from afterNavigate in the layout). */
export function noteNavigation(n: NavigationNote) {
	if (n.type === 'enter') depth = 0;
	else if (n.type === 'popstate') depth = Math.max(0, depth + (Number.isInteger(n.delta) ? (n.delta as number) : -1));
	else if (n.type !== 'form') depth++;
}

/** Whether Back can return to an earlier page of the app. */
export function canGoBack(): boolean {
	return depth > 0;
}

/** Back to the previous page of the app, or (opened directly) up to `parent`. */
export async function back(parent: string): Promise<void> {
	if (canGoBack()) {
		history.back();
		return;
	}
	await go(parent, { replaceState: true });
	depth = 0; // replaced, not added: still nothing behind
}
