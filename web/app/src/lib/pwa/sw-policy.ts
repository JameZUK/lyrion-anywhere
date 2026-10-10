// What the service worker may do with a request. It only makes the app itself start fast and
// open offline: API calls, audio, artwork and the player's files always go to the network
// and are never stored (they carry tokens, personal data, or change constantly).

export type Strategy = 'cache-first' | 'network-first' | 'bypass';

/**
 * `url` is the request; `scope` the service worker's scope (the app's base URL, which may be
 * a sub-path behind a proxy).
 */
export function strategyFor(method: string, url: URL, scope: URL): Strategy {
	if (method !== 'GET' || url.origin !== scope.origin || !url.pathname.startsWith(scope.pathname)) return 'bypass';
	if (url.search) return 'bypass'; // the app's own files never take a query
	const path = url.pathname.slice(scope.pathname.length);
	if (/^(v1|admin)(\/|$)/.test(path)) return 'bypass'; // the API, and anything like it
	if (path.startsWith('player/')) return 'bypass'; // the player worker and WebAssembly
	if (path.startsWith('_app/immutable/')) return 'cache-first'; // hashed: never change
	if (path === '' || path === 'index.html' || /^(_app\/[^/]+|icons\/[\w.-]+|favicon\.svg|manifest\.webmanifest)$/.test(path)) {
		return 'network-first';
	}
	return 'bypass';
}

/** Only complete, ordinary, same-origin answers are worth keeping. */
export const cacheable = (r: Response) => r.ok && r.status === 200 && r.type === 'basic' && !/no-store/i.test(r.headers.get('cache-control') ?? '');
