/// <reference no-default-lib="true"/>
/// <reference lib="esnext" />
/// <reference lib="webworker" />
// The app shell offline and quick to start; see $lib/pwa/sw-policy for what is never stored and
// $lib/pwa/sw-handlers for how an upgrade leaves open tabs working.
import { build, version } from '$service-worker';
import { onActivate, onFetch, onInstall, onMessage, type SwEnv } from '$lib/pwa/sw-handlers';

const sw = self as unknown as ServiceWorkerGlobalScope;
const env: SwEnv = {
	build,
	version,
	scope: () => new URL(sw.registration.scope),
	caches,
	fetch: (r) => fetch(r),
	clients: sw.clients,
	skipWaiting: () => sw.skipWaiting(),
};

sw.addEventListener('install', (event) => event.waitUntil(onInstall(env)));
sw.addEventListener('activate', (event) => event.waitUntil(onActivate(env)));
sw.addEventListener('fetch', (event) => {
	const answer = onFetch(env, event.request);
	if (answer) event.respondWith(answer); // otherwise the browser handles it as if there were no worker
});
sw.addEventListener('message', (event) => {
	const work = onMessage(env, event.data);
	if (work) event.waitUntil(work);
});
