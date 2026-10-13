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
