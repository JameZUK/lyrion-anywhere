// The service worker's install, activate, fetch and message handlers, kept apart from
// `src/service-worker/index.ts` so tests can drive them with fake caches and clients.
//
// Upgrades never pull the files out from under an open tab. A new version installs and then
// waits; the open tabs keep running the version they loaded (its lazily loaded pieces stay in
// its cache). A page lets the new version take over (`skip-waiting`) and reloads: at once
// when nothing plays on its device, otherwise when playback there stops or the person
// chooses "Reload" (see update.svelte.ts). A previous version's cache is deleted only once no
// open tab still runs it. A forced takeover with an immediate purge (the old behaviour) left
// an open tab unable to load its next page.
import { cacheable, strategyFor } from './sw-policy';

export const CACHE_PREFIX = 'anywhere-shell-';
/** Messages between the pages and the worker. */
export const MSG = {
	/** Page → worker: take over now (the page will reload). */
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
export const KEEP_UNANSWERED = 1;
/**
 * Previous versions kept at most, whatever the pages say: only the newest one. Storage is
 * tight on some devices (iPadOS home-screen apps), and an old shell left behind is worse than
 * none (see `onFetch`).
 */
export const KEEP_PREVIOUS = 1;

/** The parts of the worker's `Client` and `Clients` used here (the page's TypeScript has no worker types). */
export interface SwClient {
	postMessage(message: unknown, transfer: Transferable[]): void;
}
export interface SwClients {
	claim(): Promise<void>;
	matchAll(options: { type: 'window'; includeUncontrolled: boolean }): Promise<readonly SwClient[]>;
}

export interface SwEnv {
	/** The build's files (`immutable` from `$app/manifest`, as paths). */
	build: readonly string[];
	/** This build's version (`version` from `$app/env`). */
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

/** At most this many extra files are taken from the page (it is the app's own, but stay bounded). */
export const MAX_REFERENCED = 64;

/**
 * The app's own files the page references (`src`/`href` under `_app/`, same origin, within
 * the scope, no query), as absolute URLs. This catches `_app/boot.<hash>.js`, which is written
 * after the Vite build and so is missing from `$app/manifest`.
 */
export function referencedFiles(html: string, base: URL): string[] {
	const out = new Set<string>();
	for (const m of html.matchAll(/\s(?:src|href)\s*=\s*(?:"([^"<>]*)"|'([^'<>]*)')/gi)) {
		if (out.size >= MAX_REFERENCED) break;
		let u: URL;
		try {
			u = new URL(m[1] ?? m[2] ?? '', base);
		} catch {
			continue;
		}
		u.hash = '';
		if (!u.pathname.startsWith(`${base.pathname}_app/`) || strategyFor('GET', u, base) === 'bypass') continue;
		out.add(u.href);
	}
	return [...out];
}

/** Store `urls` in `cache`: all at once if it can, otherwise each one it can. Never throws. */
async function storeAll(cache: Cache, urls: string[]): Promise<void> {
	if (!urls.length) return;
	try {
		await cache.addAll(urls);
		return;
	} catch {
		/* one missing file, or the storage failing: try them one by one */
	}
	for (const u of urls) {
		try {
			await cache.add(u);
		} catch {
			/* left to the network */
		}
	}
}

/**
 * Install: store the new app shell, and every one of the app's files its page references. It
 * then waits for the page to let it take over (see update.svelte.ts). Storing is best-effort:
 * the install completes even if nothing could be stored (a failing Cache API, as WebKit's
 * home-screen apps can have, must not keep a new version from ever installing; every request
 * then simply goes to the network).
 */
export async function onInstall(env: SwEnv): Promise<void> {
	const base = env.scope();
	const shell = [...env.build, '', 'manifest.webmanifest', 'favicon.svg', 'icons/icon-192.png', 'boot-guard.js']
		.map((p) => new URL(p.replace(/^\//, ''), base).href)
		.filter((u) => strategyFor('GET', new URL(u), base) !== 'bypass');
	try {
		const cache = await env.caches.open(cacheName(env));
		await storeAll(cache, shell);
		const page = await cache.match(base.href);
		if (!page) return;
		const have = new Set(shell);
		await storeAll(
			cache,
			referencedFiles(await page.text(), base).filter((u) => !have.has(u)),
		);
	} catch {
		/* the storage failed: this version runs from the network */
	}
}

/** Activate: control the pages, and drop the previous versions no open tab needs. Never rejects. */
export async function onActivate(env: SwEnv): Promise<void> {
	try {
		await env.clients.claim();
	} catch {
		/* the pages pick it up on their next load */
	}
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
 * Delete the shell caches of previous versions. Only the newest previous version may stay,
 * and only while an open tab runs it or a tab did not answer (a frozen tab may still need
 * it; the next page start tidies again); any older one goes whatever the pages say. Never
 * rejects: a failing Cache API leaves things as they are.
 */
export async function prune(env: SwEnv): Promise<string[]> {
	const deleted: string[] = [];
	try {
		const current = cacheName(env);
		const old = (await env.caches.keys()).filter((k) => k.startsWith(CACHE_PREFIX) && k !== current);
		if (!old.length) return [];
		const pages = await env.clients.matchAll({ type: 'window', includeUncontrolled: true });
		const answers = await Promise.all(pages.map((c) => ask(env, c)));
		const inUse = new Set(answers.filter((v): v is string => v !== null));
		const unanswered = answers.some((v) => v === null);
		const keepable = new Set(old.slice(-KEEP_PREVIOUS));
		const keepNewest = unanswered ? new Set(old.slice(-KEEP_UNANSWERED)) : new Set<string>();
		for (const key of old) {
			if (keepable.has(key) && (inUse.has(versionOf(key)) || keepNewest.has(key))) continue;
			try {
				await env.caches.delete(key);
				deleted.push(key);
			} catch {
				/* tried again on the next page start */
			}
		}
	} catch {
		/* the storage failed: tidied on the next page start */
	}
	return deleted;
}

/**
 * The stored copy of a request: this version's first, then a previous version's (newest
 * first). Undefined when there is none, or when the Cache API fails.
 */
async function stored(env: SwEnv, request: Request | string): Promise<Response | undefined> {
	try {
		const current = cacheName(env);
		const keys = (await env.caches.keys()).filter((k) => k.startsWith(CACHE_PREFIX) && k !== current).reverse();
		for (const key of [current, ...keys]) {
			if (!(await env.caches.has(key))) continue;
			const hit = await (await env.caches.open(key)).match(request);
			if (hit) return hit;
		}
	} catch {
		/* the storage failed: as if nothing were stored */
	}
	return undefined;
}

/** Keep a copy of a fresh answer, best-effort: a failing write never changes the answer. */
async function keep(env: SwEnv, request: Request, copy: Response): Promise<void> {
	try {
		await (await env.caches.open(cacheName(env))).put(request, copy);
	} catch {
		/* the storage failed: the answer stands, uncached */
	}
}

/**
 * Fetch: what to answer, or null to leave the request to the browser (as if there were no
 * worker). An open tab of the previous version still finds its own hashed files here, after
 * the server has stopped serving them.
 *
 * Whenever the network answers, that answer is what the page gets. The stored copy (for a
 * page load, the stored app shell) is used only when the network itself failed. Before, a
 * failing cache write fell into the same "offline" path: the page got an old stored
 * index.html whose start-up script the server no longer had, and the installed app showed a
 * white screen on every launch. `waitUntil` (the fetch event's) keeps the worker alive for the
 * cache write.
 */
export function onFetch(env: SwEnv, request: Request, waitUntil?: (p: Promise<unknown>) => void): Promise<Response> | null {
	const url = new URL(request.url);
	const strategy = strategyFor(request.method, url, env.scope());
	if (strategy === 'bypass') return null;
	return (async () => {
		if (strategy === 'cache-first') {
			const hit = await stored(env, request);
			if (hit) return hit;
		}
		let fresh: Response;
		try {
			fresh = await env.fetch(request);
		} catch (e) {
			// Offline: the stored copy (for a page load, the app shell).
			const copy = (await stored(env, request)) ?? (request.mode === 'navigate' ? await stored(env, env.scope().href) : undefined);
			if (copy) return copy;
			throw e;
		}
		if (cacheable(fresh)) {
			let write: Promise<void> | undefined;
			try {
				write = keep(env, request, fresh.clone());
			} catch {
				/* the body could not be copied: not stored */
			}
			if (write) {
				try {
					waitUntil?.(write);
				} catch {
					/* the event is over: the write carries on regardless */
				}
			}
		}
		return fresh;
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
