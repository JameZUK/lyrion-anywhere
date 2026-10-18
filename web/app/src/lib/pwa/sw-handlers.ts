// The service worker's install, activate, fetch and message handlers, kept apart from
// `src/service-worker.ts` so tests can drive them with fake caches and clients.
//
// Upgrades never pull the files out from under an open tab. A new version installs and then
// waits; the open tabs keep running the version they loaded (its lazily loaded pieces stay in
// its cache) and are offered a reload ("A new version is ready"). Only when the person
// accepts does the new version take over (`skip-waiting`), and a previous version's cache is
// deleted only once no open tab still runs it. A forced takeover with an immediate purge (the
// old behaviour) left an open tab unable to load its next page, and a full reload stopped
// this device's playback.
import { cacheable, strategyFor } from './sw-policy';

export const CACHE_PREFIX = 'anywhere-shell-';
/** Messages between the pages and the worker. */
export const MSG = {
	/** Page → worker: take over now (the person chose to reload). */
	skipWaiting: 'anywhere-skip-waiting',
	/** Page → worker: a page has started; tidy the caches no tab needs any more. */
	hello: 'anywhere-hello',
	/** Worker → page (with a reply port): which version are you running? */
	ask: 'anywhere-version?',
	/** Page → worker, on that port: the version it runs. */
	reply: 'anywhere-version',
} as const;
/** How long a page has to say which version it runs. A frozen background tab may never answer. */
export const ASK_TIMEOUT_MS = 2000;
/** Previous versions kept for tabs that did not answer (a frozen tab may still need one). */
export const KEEP_UNANSWERED = 2;

/** The parts of the worker's `Client` and `Clients` used here (the page's TypeScript has no worker types). */
export interface SwClient {
	postMessage(message: unknown, transfer: Transferable[]): void;
}
export interface SwClients {
	claim(): Promise<void>;
	matchAll(options: { type: 'window'; includeUncontrolled: boolean }): Promise<readonly SwClient[]>;
}

export interface SwEnv {
	/** The build's files (`$service-worker`'s `build`). */
	build: readonly string[];
	/** This build's version (`$service-worker`'s `version`). */
	version: string;
	scope(): URL;
	caches: CacheStorage;
	fetch: (r: Request) => Promise<Response>;
	clients: SwClients;
	skipWaiting(): Promise<void>;
	/** A message channel (tests may give a fake). */
	channel?: () => MessageChannel;
}

const cacheName = (env: SwEnv) => `${CACHE_PREFIX}${env.version}`;
const versionOf = (key: string) => key.slice(CACHE_PREFIX.length);
/** A version as a page reports it: a short plain string, anything else is ignored. */
const validVersion = (v: unknown): v is string => typeof v === 'string' && v.length > 0 && v.length <= 64 && /^[\w.-]+$/.test(v);

/** Install: store the new app shell. It then waits; it never takes over by itself. */
export async function onInstall(env: SwEnv): Promise<void> {
	const cache = await env.caches.open(cacheName(env));
	const base = env.scope();
	const shell = [...env.build, '', 'manifest.webmanifest', 'favicon.svg', 'icons/icon-192.png'].map((p) => new URL(p.replace(/^\//, ''), base).href);
	await cache.addAll(shell.filter((u) => strategyFor('GET', new URL(u), base) !== 'bypass'));
}

/** Activate: control the pages, and drop the previous versions no open tab needs. */
export async function onActivate(env: SwEnv): Promise<void> {
	await env.clients.claim();
	await prune(env);
}

/** Ask one page which version it runs; null when it does not answer in time. */
function ask(env: SwEnv, client: SwClient): Promise<string | null> {
	return new Promise((resolve) => {
		let channel: MessageChannel;
		try {
			channel = env.channel ? env.channel() : new MessageChannel();
		} catch {
			resolve(null);
			return;
		}
		const timer = setTimeout(() => done(null), ASK_TIMEOUT_MS);
		function done(v: string | null) {
			clearTimeout(timer);
			channel.port1.onmessage = null;
			try {
				channel.port1.close();
			} catch {
				/* already closed */
			}
			resolve(v);
		}
		channel.port1.onmessage = (e: MessageEvent) => {
			const d = e.data as { type?: unknown; version?: unknown } | null;
			done(d && typeof d === 'object' && d.type === MSG.reply && validVersion(d.version) ? d.version : null);
		};
		try {
			client.postMessage({ type: MSG.ask }, [channel.port2]);
		} catch {
			done(null);
		}
	});
}

/**
 * Delete the shell caches of previous versions that no open tab runs. A tab that does not
 * answer might run any of them, so then the newest few previous versions stay (the next
 * page start tidies again).
 */
export async function prune(env: SwEnv): Promise<string[]> {
	const current = cacheName(env);
	const old = (await env.caches.keys()).filter((k) => k.startsWith(CACHE_PREFIX) && k !== current);
	if (!old.length) return [];
	const pages = await env.clients.matchAll({ type: 'window', includeUncontrolled: true });
	const answers = await Promise.all(pages.map((c) => ask(env, c)));
	const inUse = new Set(answers.filter((v): v is string => v !== null));
	const unanswered = answers.some((v) => v === null);
	const keepNewest = unanswered ? new Set(old.slice(-KEEP_UNANSWERED)) : new Set<string>();
	const deleted: string[] = [];
	for (const key of old) {
		if (inUse.has(versionOf(key)) || keepNewest.has(key)) continue;
		await env.caches.delete(key);
		deleted.push(key);
	}
	return deleted;
}

/** The stored copy of a request: this version's first, then a previous version's (newest first). */
async function stored(env: SwEnv, request: Request | string): Promise<Response | undefined> {
	const current = cacheName(env);
	const keys = (await env.caches.keys()).filter((k) => k.startsWith(CACHE_PREFIX) && k !== current).reverse();
	for (const key of [current, ...keys]) {
		if (!(await env.caches.has(key))) continue;
		const hit = await (await env.caches.open(key)).match(request);
		if (hit) return hit;
	}
	return undefined;
}

/**
 * Fetch: what to answer, or null to leave the request to the browser (as if there were no
 * worker). An open tab of the previous version still finds its own hashed files here, after
 * the server has stopped serving them.
 */
export function onFetch(env: SwEnv, request: Request): Promise<Response> | null {
	const url = new URL(request.url);
	const strategy = strategyFor(request.method, url, env.scope());
	if (strategy === 'bypass') return null;
	return (async () => {
		if (strategy === 'cache-first') {
			const hit = await stored(env, request);
			if (hit) return hit;
		}
		try {
			const fresh = await env.fetch(request);
			if (cacheable(fresh)) await (await env.caches.open(cacheName(env))).put(request, fresh.clone());
			return fresh;
		} catch (e) {
			// Offline: the stored copy (for a page load, the app shell).
			const copy = (await stored(env, request)) ?? (request.mode === 'navigate' ? await stored(env, env.scope().href) : undefined);
			if (copy) return copy;
			throw e;
		}
	})();
}

/** Messages from the pages: the reload the person chose, and "a page has started". */
export function onMessage(env: SwEnv, data: unknown): Promise<unknown> | null {
	if (!data || typeof data !== 'object') return null;
	const type = (data as { type?: unknown }).type;
	if (type === MSG.skipWaiting) return env.skipWaiting();
	if (type === MSG.hello) return prune(env);
	return null;
}
