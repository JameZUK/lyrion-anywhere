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
