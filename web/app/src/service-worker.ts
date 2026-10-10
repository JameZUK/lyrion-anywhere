/// <reference no-default-lib="true"/>
/// <reference lib="esnext" />
/// <reference lib="webworker" />
// The app shell offline and quick to start; see $lib/pwa/sw-policy for what is never stored.
import { build, version } from '$service-worker';
import { cacheable, strategyFor } from '$lib/pwa/sw-policy';

const sw = self as unknown as ServiceWorkerGlobalScope;
const CACHE = `anywhere-shell-${version}`;
const scope = () => new URL(sw.registration.scope);

sw.addEventListener('install', (event) => {
	event.waitUntil(
		(async () => {
			const cache = await caches.open(CACHE);
			const base = scope();
			const shell = [...build, '', 'manifest.webmanifest', 'favicon.svg', 'icons/icon-192.png'].map((p) => new URL(p.replace(/^\//, ''), base).href);
			await cache.addAll(shell.filter((u) => strategyFor('GET', new URL(u), base) !== 'bypass'));
			await sw.skipWaiting();
		})(),
	);
});

sw.addEventListener('activate', (event) => {
	event.waitUntil(
		(async () => {
			for (const key of await caches.keys()) if (key !== CACHE) await caches.delete(key);
			await sw.clients.claim();
		})(),
	);
});

sw.addEventListener('fetch', (event) => {
	const url = new URL(event.request.url);
	const strategy = strategyFor(event.request.method, url, scope());
	if (strategy === 'bypass') return; // the browser handles it as if there were no worker
	event.respondWith(
		(async () => {
			const cache = await caches.open(CACHE);
			if (strategy === 'cache-first') {
				const hit = await cache.match(event.request);
				if (hit) return hit;
			}
			try {
				const fresh = await fetch(event.request);
				if (cacheable(fresh)) await cache.put(event.request, fresh.clone());
				return fresh;
			} catch (e) {
				// Offline: the stored copy (for a page load, the app shell).
				const stored = (await cache.match(event.request)) ?? (event.request.mode === 'navigate' ? await cache.match(scope().href) : undefined);
				if (stored) return stored;
				throw e;
			}
		})(),
	);
});
