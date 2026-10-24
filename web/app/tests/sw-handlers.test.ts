// The service worker's handlers, in a fake caches/clients world. An upgrade must leave an
// open tab working: the new version waits, the previous version's files stay until no open
// tab runs it, and the person chooses when to reload.
import { ASK_TIMEOUT_MS, CACHE_PREFIX, KEEP_PREVIOUS, KEEP_UNANSWERED, MAX_REFERENCED, MSG, onActivate, onFetch, onInstall, onMessage, prune, referencedFiles, type SwClient, type SwEnv } from '#lib/pwa/sw-handlers';

const SCOPE = 'https://music.example.com/lms/';

/** What the fake network serves to `addAll`: a body per URL, and URLs that are missing (404). */
const served = { bodies: new Map<string, string>(), missing: new Set<string>(), added: [] as string[][] };
beforeEach(() => {
	served.bodies.clear();
	served.missing.clear();
	served.added = [];
});

class FakeCache {
	entries = new Map<string, Response>();
	async match(r: Request | string) {
		const hit = this.entries.get(typeof r === 'string' ? r : r.url);
		return hit ? hit.clone() : undefined;
	}
	async put(r: Request | string, res: Response) {
		this.entries.set(typeof r === 'string' ? r : r.url, res);
	}
	async addAll(urls: string[]) {
		served.added.push(urls);
		// All or nothing, like the real one.
		const bad = urls.find((u) => served.missing.has(u));
		if (bad) throw new TypeError(`404 ${bad}`);
		for (const u of urls) this.entries.set(u, new Response(served.bodies.get(u) ?? `stored ${u}`));
	}
	async add(u: string) {
		await this.addAll([u]);
	}
}

class FakeCaches {
	stores = new Map<string, FakeCache>();
	async open(name: string) {
		let c = this.stores.get(name);
		if (!c) this.stores.set(name, (c = new FakeCache()));
		return c;
	}
	async has(name: string) {
		return this.stores.has(name);
	}
	async keys() {
		return [...this.stores.keys()];
	}
	async delete(name: string) {
		return this.stores.delete(name);
	}
	async match() {
		throw new Error('the handlers look in their own caches only');
	}
}

/** A fake message channel: whatever the page posts on port2 reaches port1 straight away. */
function channel(): MessageChannel {
	const port1 = { onmessage: null as ((e: { data: unknown }) => void) | null, close() {} };
	const port2 = { postMessage: (d: unknown) => port1.onmessage?.({ data: d }) };
	return { port1, port2 } as unknown as MessageChannel;
}

/** An open page: answers the version question (or not, when `version` is null). */
function page(version: string | null | (() => unknown)): SwClient & { got: unknown[] } {
	const got: unknown[] = [];
	return {
		got,
		postMessage(m: unknown, transfer: Transferable[]) {
			got.push(m);
			if (version === null) return; // a frozen background tab
			const port = transfer[0] as unknown as { postMessage(d: unknown): void };
			if ((m as { type: string }).type === MSG.ask) port.postMessage(typeof version === 'function' ? version() : { type: MSG.reply, version });
		},
	};
}

function world(version: string, pages: SwClient[] = []) {
	const caches = new FakeCaches();
	const calls = { claim: 0, skipWaiting: 0, fetched: [] as string[] };
	let network: (r: Request) => Promise<Response> = async (r) => basic(`fresh ${r.url}`);
	const env: SwEnv = {
		build: ['/_app/immutable/entry/start.v2hash.js', '/_app/version.json'],
		version,
		scope: () => new URL(SCOPE),
		caches: caches as unknown as CacheStorage,
		fetch: (r) => {
			calls.fetched.push(r.url);
			return network(r);
		},
		clients: {
			claim: async () => void calls.claim++,
			matchAll: async () => pages,
		},
		skipWaiting: async () => void calls.skipWaiting++,
		channel,
	};
	return { env, caches, calls, setNetwork: (f: typeof network) => (network = f) };
}

function basic(body: string, status = 200) {
	const r = new Response(body, { status });
	Object.defineProperty(r, 'type', { value: 'basic' });
	return r;
}

const req = (path: string, mode: RequestMode = 'cors') => ({ url: new URL(path, SCOPE).href, method: 'GET', mode }) as unknown as Request;

describe('install', () => {
	test('stores the new shell and waits: it never takes over by itself', async () => {
		const w = world('v2');
		await (await w.caches.open(`${CACHE_PREFIX}v1`)).put(`${SCOPE}_app/immutable/nodes/3.v1hash.js`, new Response('old'));
		await onInstall(w.env);
		expect(w.calls.skipWaiting).toBe(0);
		const shell = [...w.caches.stores.get(`${CACHE_PREFIX}v2`)!.entries.keys()];
		expect(shell).toContain(`${SCOPE}_app/immutable/entry/start.v2hash.js`);
		expect(shell).toContain(SCOPE);
		expect(shell).toContain(`${SCOPE}manifest.webmanifest`);
		// The previous version's files are untouched while it still runs.
		expect(await w.caches.has(`${CACHE_PREFIX}v1`)).toBe(true);
	});

	test("SvelteKit 3's build paths (relative to the base, no leading slash) are stored under the worker's scope", async () => {
		const w = world('v3');
		// What src/service-worker/index.ts passes: `immutable` from $app/manifest, mapped to its paths.
		const immutable = [{ path: '_app/immutable/entry/start.v3hash.js' }, { path: '_app/immutable/assets/0.v3hash.css' }];
		w.env.build = immutable.map((f) => f.path);
		await onInstall(w.env);
		const shell = [...w.caches.stores.get(`${CACHE_PREFIX}v3`)!.entries.keys()];
		expect(shell).toContain(`${SCOPE}_app/immutable/entry/start.v3hash.js`);
		expect(shell).toContain(`${SCOPE}_app/immutable/assets/0.v3hash.css`);
		// Never resolved against the origin's root, outside the app's base path.
		expect(shell.filter((u) => !u.startsWith(SCOPE))).toEqual([]);
	});
});

describe('install: the files the page references', () => {
	const PAGE = `<!doctype html><html><head>
		<script src="./boot-guard.js"></script>
		<link rel="modulepreload" href="./_app/immutable/entry/start.v2hash.js">
		<link rel="stylesheet" href='./_app/immutable/assets/0.v2hash.css'>
		<link rel="icon" href="./favicon.svg">
		</head><body><div id="app"></div>
		<script src="./_app/boot.0123456789abcdef.js"></script>
		</body></html>`;

	test('the externalised start-up script (not in the build manifest) and the boot guard are stored', async () => {
		const w = world('v2');
		served.bodies.set(SCOPE, PAGE);
		await onInstall(w.env);
		const shell = [...w.caches.stores.get(`${CACHE_PREFIX}v2`)!.entries.keys()];
		expect(shell).toContain(`${SCOPE}_app/boot.0123456789abcdef.js`);
		expect(shell).toContain(`${SCOPE}_app/immutable/assets/0.v2hash.css`);
		expect(shell).toContain(`${SCOPE}boot-guard.js`);
		// What the shell list already had is not fetched twice.
		expect(served.added[1]).toEqual([`${SCOPE}_app/immutable/assets/0.v2hash.css`, `${SCOPE}_app/boot.0123456789abcdef.js`]);
	});

	test('a page with nothing more to fetch adds nothing', async () => {
		const w = world('v2');
		served.bodies.set(SCOPE, '<html><script src="./_app/immutable/entry/start.v2hash.js"></script></html>');
		await onInstall(w.env);
		expect(served.added).toHaveLength(1);
	});

	test('a file that cannot be fetched still lets the install complete, with everything else stored', async () => {
		const w = world('v2');
		served.bodies.set(SCOPE, PAGE);
		served.missing.add(`${SCOPE}_app/boot.0123456789abcdef.js`);
		served.missing.add(`${SCOPE}favicon.svg`);
		await onInstall(w.env);
		const shell = [...w.caches.stores.get(`${CACHE_PREFIX}v2`)!.entries.keys()];
		expect(shell).toContain(SCOPE);
		expect(shell).toContain(`${SCOPE}_app/immutable/assets/0.v2hash.css`);
		expect(shell).toContain(`${SCOPE}boot-guard.js`);
		expect(shell).not.toContain(`${SCOPE}favicon.svg`);
		expect(shell).not.toContain(`${SCOPE}_app/boot.0123456789abcdef.js`);
	});

	test('only the app’s own files under _app/: other origins, paths outside the scope, queries and the player are refused', () => {
		const base = new URL(SCOPE);
		const html = [
			'<script src="https://cdn.example.org/lms/_app/evil.js"></script>',
			'<script src="//cdn.example.org/lms/_app/evil.js"></script>',
			'<script src="/_app/root.js"></script>', // the origin's root, outside /lms/
			'<script src="../other/_app/x.js"></script>',
			'<script src="./_app/../v1/secret"></script>', // normalised: the API
			'<script src="./_app/x.js?token=abc"></script>',
			'<script src="./player/_app/x.js"></script>',
			'<a href="javascript:alert(1)">x</a>',
			'<img src="http://[broken">',
			'<link href="./manifest.webmanifest">', // the shell, but not under _app/
			'<script data-src="./_app/not-an-attribute.js"></script>',
			'<script src="./_app/ok.js#frag"></script>',
			"<script src='./_app/single.js'></script>",
			'<script src = "./_app/spaced.js"></script>',
			'<script src="./_app/ok.js"></script>', // a duplicate
		].join('\n');
		expect(referencedFiles(html, base)).toEqual([`${SCOPE}_app/ok.js`, `${SCOPE}_app/single.js`, `${SCOPE}_app/spaced.js`]);
	});

	test('bounded, and never throws on arbitrary input', () => {
		const base = new URL(SCOPE);
		const many = Array.from({ length: MAX_REFERENCED * 3 }, (_, i) => `<script src="./_app/f${i}.js"></script>`).join('');
		expect(referencedFiles(many, base)).toHaveLength(MAX_REFERENCED);
		const chars = ' <>"\'=srchef./_app:%\\\n\0 ';
		let seed = 7;
		const rnd = () => (seed = (seed * 1103515245 + 12345) % 2 ** 31) / 2 ** 31;
		for (let i = 0; i < 500; i++) {
			const junk = Array.from({ length: Math.floor(rnd() * 200) }, () => chars[Math.floor(rnd() * chars.length)]).join('');
			for (const u of referencedFiles(junk, base)) expect(u.startsWith(`${SCOPE}_app/`)).toBe(true);
		}
	});
});

describe('activate', () => {
	async function withOld(w: ReturnType<typeof world>, ...versions: string[]) {
		for (const v of versions) await w.caches.open(`${CACHE_PREFIX}${v}`);
		await w.caches.open(`${CACHE_PREFIX}${w.env.version}`);
		await w.caches.open('someone-elses-cache');
	}

	test('an open tab still running the previous version keeps its files', async () => {
		const w = world('v2', [page('v1'), page('v2')]);
		await withOld(w, 'v0', 'v1');
		await onActivate(w.env);
		expect(w.calls.claim).toBe(1);
		expect(await w.caches.keys()).toEqual([`${CACHE_PREFIX}v1`, `${CACHE_PREFIX}v2`, 'someone-elses-cache']);
	});

	test('once every tab runs the new version, the previous versions go (and nothing else)', async () => {
		const w = world('v2', [page('v2'), page('v2')]);
		await withOld(w, 'v0', 'v1');
		await onActivate(w.env);
		expect(await w.caches.keys()).toEqual([`${CACHE_PREFIX}v2`, 'someone-elses-cache']);
	});

	test('no open tabs: the previous versions go', async () => {
		const w = world('v2', []);
		await withOld(w, 'v1');
		expect(await prune(w.env)).toEqual([`${CACHE_PREFIX}v1`]);
	});

	test('a tab that never answers: the newest previous version stays, older ones go', async () => {
		vi.useFakeTimers();
		try {
			const w = world('v9', [page(null)]);
			await withOld(w, 'v5', 'v6', 'v7', 'v8');
			const done = onActivate(w.env);
			await vi.advanceTimersByTimeAsync(ASK_TIMEOUT_MS + 10);
			await done;
			const left = (await w.caches.keys()).filter((k) => k.startsWith(CACHE_PREFIX));
			expect(left).toEqual([`${CACHE_PREFIX}v8`, `${CACHE_PREFIX}v9`]);
			expect(KEEP_UNANSWERED).toBe(1);
		} finally {
			vi.useRealTimers();
		}
	});

	test('a malformed answer counts as no answer (it cannot name a cache to keep or delete)', async () => {
		const w = world('v3', [page(() => ({ type: MSG.reply, version: '../../v1' })), page(() => ({ type: 'other', version: 'v1' })), page(() => 'v1')]);
		await withOld(w, 'v0', 'v1', 'v2');
		await onActivate(w.env);
		const left = (await w.caches.keys()).filter((k) => k.startsWith(CACHE_PREFIX));
		expect(left).toEqual([`${CACHE_PREFIX}v2`, `${CACHE_PREFIX}v3`]);
	});

	test('only the newest previous version can stay: an older one goes even while a tab runs it', async () => {
		const w = world('v3', [page('v0'), page('v2')]);
		await withOld(w, 'v0', 'v1', 'v2');
		await onActivate(w.env);
		expect(await w.caches.keys()).toEqual([`${CACHE_PREFIX}v2`, `${CACHE_PREFIX}v3`, 'someone-elses-cache']);
		expect(KEEP_PREVIOUS).toBe(1);
	});

	test('a page that throws on postMessage does not stop the activation', async () => {
		const bad: SwClient = {
			postMessage() {
				throw new Error('gone');
			},
		};
		const w = world('v2', [bad]);
		await withOld(w, 'v1');
		await expect(onActivate(w.env)).resolves.toBeUndefined();
	});
});

describe('fetch', () => {
	test('the API, audio and other origins are left to the browser', () => {
		const w = world('v2');
		for (const p of ['v1/rpc', 'v1/stream/abc', 'player/player-worker.js', 'https://cdn.example.org/x.js']) expect(onFetch(w.env, req(p)), p).toBeNull();
		expect(onFetch(w.env, { ...req(''), method: 'POST' } as Request)).toBeNull();
	});

	test('after an upgrade an open tab still gets its previous version’s chunks, without the network', async () => {
		const w = world('v2');
		await (await w.caches.open(`${CACHE_PREFIX}v1`)).put(`${SCOPE}_app/immutable/nodes/3.v1hash.js`, new Response('old chunk'));
		await w.caches.open(`${CACHE_PREFIX}v2`);
		const res = await onFetch(w.env, req('_app/immutable/nodes/3.v1hash.js'))!;
		expect(await res.text()).toBe('old chunk');
		expect(w.calls.fetched).toEqual([]);
	});

	test('a hashed file not stored anywhere is fetched and kept in this version’s cache', async () => {
		const w = world('v2');
		const writes: Promise<unknown>[] = [];
		const res = await onFetch(w.env, req('_app/immutable/nodes/4.v2hash.js'), (p) => writes.push(p))!;
		expect(await res.text()).toBe(`fresh ${SCOPE}_app/immutable/nodes/4.v2hash.js`);
		expect(writes).toHaveLength(1); // the event is kept alive for the write
		await Promise.all(writes);
		expect(w.caches.stores.get(`${CACHE_PREFIX}v2`)!.entries.has(`${SCOPE}_app/immutable/nodes/4.v2hash.js`)).toBe(true);
	});

	test('the page itself is network-first; an error answer is passed on but not stored', async () => {
		const w = world('v2');
		await (await w.caches.open(`${CACHE_PREFIX}v2`)).put(SCOPE, new Response('stored shell'));
		w.setNetwork(async () => basic('oops', 500));
		const res = await onFetch(w.env, req('', 'navigate'))!;
		expect(res.status).toBe(500);
		expect(await (await w.caches.open(`${CACHE_PREFIX}v2`)).match(SCOPE).then((r) => r!.text())).toBe('stored shell');
	});

	test('offline: a page load gets the stored shell, from a previous version if need be', async () => {
		const w = world('v2');
		await (await w.caches.open(`${CACHE_PREFIX}v1`)).put(SCOPE, new Response('v1 shell'));
		w.setNetwork(async () => {
			throw new TypeError('offline');
		});
		const res = await onFetch(w.env, req('index.html', 'navigate'))!;
		expect(await res.text()).toBe('v1 shell');
	});

	test('offline with nothing stored: the failure is passed on', async () => {
		const w = world('v2');
		w.setNetwork(async () => {
			throw new TypeError('offline');
		});
		await expect(onFetch(w.env, req('_app/version.json'))!).rejects.toThrow('offline');
	});
});

// Regression: a failing Cache API (WebKit's home-screen apps: storage errors, eviction, quota)
// was treated as "offline", so a page load got the OLD stored index.html, whose start-up
// script the server no longer had: a white screen on every launch. A new worker could not
// install either (its addAll failed the same way), so nothing ever replaced the old one.
describe('a failing Cache API never changes what the network answered', () => {
	const fail = (what: string) => async () => {
		throw new DOMException(`${what} failed`, 'UnknownError');
	};

	/** A world whose previous version stored an old shell, and whose cache then breaks. */
	async function broken(how: 'put' | 'open' | 'all') {
		const w = world('v2');
		const old = await w.caches.open(`${CACHE_PREFIX}v1`);
		await old.put(SCOPE, new Response('old shell (references _app/boot.old.js)'));
		await old.put(`${SCOPE}_app/immutable/nodes/4.v1hash.js`, new Response('old chunk'));
		const current = await w.caches.open(`${CACHE_PREFIX}v2`);
		if (how === 'put' || how === 'all') current.put = fail('put');
		if (how === 'open' || how === 'all') w.caches.open = fail('open');
		if (how === 'all') {
			w.caches.keys = fail('keys');
			w.caches.has = fail('has');
			w.caches.delete = fail('delete');
		}
		w.setNetwork(async (r) => basic(`fresh ${r.url}`));
		return w;
	}

	test('put() rejects: a page load gets the fresh page, not the stored one', async () => {
		const w = await broken('put');
		const writes: Promise<unknown>[] = [];
		const res = await onFetch(w.env, req('', 'navigate'), (p) => writes.push(p))!;
		expect(await res.text()).toBe(`fresh ${SCOPE}`);
		await expect(Promise.all(writes)).resolves.toBeDefined(); // the failed write is quiet
	});

	test('put() rejects: other shell files are the fresh ones too', async () => {
		const w = await broken('put');
		const res = await onFetch(w.env, req('manifest.webmanifest'))!;
		expect(await res.text()).toBe(`fresh ${SCOPE}manifest.webmanifest`);
	});

	test('open() rejects: a page load gets the fresh page', async () => {
		const w = await broken('open');
		const res = await onFetch(w.env, req('', 'navigate'))!;
		expect(await res.text()).toBe(`fresh ${SCOPE}`);
	});

	test('open() rejects: a hashed file falls through to the network', async () => {
		const w = await broken('open');
		const res = await onFetch(w.env, req('_app/immutable/nodes/4.v1hash.js'))!;
		expect(await res.text()).toBe(`fresh ${SCOPE}_app/immutable/nodes/4.v1hash.js`);
	});

	test('keys()/has()/open() all reject: hashed files and pages still come from the network', async () => {
		const w = await broken('all');
		expect(await (await onFetch(w.env, req('_app/immutable/nodes/5.v2hash.js'))!).text()).toBe(`fresh ${SCOPE}_app/immutable/nodes/5.v2hash.js`);
		expect(await (await onFetch(w.env, req('', 'navigate'))!).text()).toBe(`fresh ${SCOPE}`);
	});

	test('a response whose body cannot be copied is still returned', async () => {
		const w = world('v2');
		w.setNetwork(async () => {
			const r = basic('fresh page');
			r.clone = () => {
				throw new TypeError('body used');
			};
			return r;
		});
		const res = await onFetch(w.env, req('', 'navigate'))!;
		expect(await res.text()).toBe('fresh page');
	});

	test('the network failing (offline) still gets the stored copy, as before', async () => {
		const w = await broken('put');
		w.setNetwork(async () => {
			throw new TypeError('offline');
		});
		expect(await (await onFetch(w.env, req('', 'navigate'))!).text()).toBe('old shell (references _app/boot.old.js)');
		expect(await (await onFetch(w.env, req('_app/immutable/nodes/4.v1hash.js'))!).text()).toBe('old chunk');
	});

	test('offline with the Cache API failing too: the network error is passed on', async () => {
		const w = await broken('all');
		w.setNetwork(async () => {
			throw new TypeError('offline');
		});
		await expect(onFetch(w.env, req('', 'navigate'))!).rejects.toThrow('offline');
	});

	test('addAll() rejects: the install still completes, and later fetches work', async () => {
		const w = world('v2');
		const cache = await w.caches.open(`${CACHE_PREFIX}v2`);
		cache.addAll = fail('addAll');
		cache.add = fail('add');
		await expect(onInstall(w.env)).resolves.toBeUndefined();
		expect(await (await onFetch(w.env, req('', 'navigate'))!).text()).toBe(`fresh ${SCOPE}`);
	});

	test('open() rejects: the install still completes', async () => {
		const w = await broken('all');
		await expect(onInstall(w.env)).resolves.toBeUndefined();
	});

	test('prune and activate resolve with a failing Cache API', async () => {
		const w = await broken('all');
		await expect(prune(w.env)).resolves.toEqual([]);
		await expect(onActivate(w.env)).resolves.toBeUndefined();
		await expect(onMessage(w.env, { type: MSG.hello })).resolves.toEqual([]);
		// Only delete() failing: the rest is still tidied, nothing thrown.
		const v = world('v3');
		for (const name of ['v1', 'v2', 'v3']) await v.caches.open(`${CACHE_PREFIX}${name}`);
		const realDelete = v.caches.delete.bind(v.caches);
		v.caches.delete = async (name: string) => {
			if (name.endsWith('v1')) throw new DOMException('delete failed', 'UnknownError');
			return realDelete(name);
		};
		expect(await prune(v.env)).toEqual([`${CACHE_PREFIX}v2`]);
	});

	test('a failing claim() does not stop the tidy', async () => {
		const w = world('v2');
		await w.caches.open(`${CACHE_PREFIX}v1`);
		w.env.clients.claim = fail('claim');
		await expect(onActivate(w.env)).resolves.toBeUndefined();
		expect(await w.caches.has(`${CACHE_PREFIX}v1`)).toBe(false);
	});
});

describe('messages', () => {
	test('"reload now" from a page lets the waiting version take over', async () => {
		const w = world('v2');
		await onMessage(w.env, { type: MSG.skipWaiting });
		expect(w.calls.skipWaiting).toBe(1);
	});

	test('a page starting tidies the caches no tab needs', async () => {
		const w = world('v2', [page('v2')]);
		await w.caches.open(`${CACHE_PREFIX}v1`);
		await w.caches.open(`${CACHE_PREFIX}v2`);
		await onMessage(w.env, { type: MSG.hello });
		expect(await w.caches.keys()).toEqual([`${CACHE_PREFIX}v2`]);
	});

	test('anything else is ignored', () => {
		const w = world('v2');
		for (const m of [null, undefined, 'anywhere-skip-waiting', 42, {}, { type: 'other' }, { type: { toString: () => MSG.skipWaiting } }]) expect(onMessage(w.env, m)).toBeNull();
		expect(w.calls.skipWaiting).toBe(0);
	});
});
