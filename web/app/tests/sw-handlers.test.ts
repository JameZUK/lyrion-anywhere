// The service worker's handlers, in a fake caches/clients world. An upgrade must leave an
// open tab working: the new version waits, the previous version's files stay until no open
// tab runs it, and the person chooses when to reload.
import { ASK_TIMEOUT_MS, CACHE_PREFIX, KEEP_UNANSWERED, MSG, onActivate, onFetch, onInstall, onMessage, prune, type SwClient, type SwEnv } from '$lib/pwa/sw-handlers';

const SCOPE = 'https://music.example.com/lms/';

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
		for (const u of urls) this.entries.set(u, new Response(`stored ${u}`));
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

	test('a tab that never answers: the newest previous versions stay, older ones go', async () => {
		vi.useFakeTimers();
		try {
			const w = world('v9', [page(null)]);
			await withOld(w, 'v5', 'v6', 'v7', 'v8');
			const done = onActivate(w.env);
			await vi.advanceTimersByTimeAsync(ASK_TIMEOUT_MS + 10);
			await done;
			const left = (await w.caches.keys()).filter((k) => k.startsWith(CACHE_PREFIX));
			expect(left).toEqual([`${CACHE_PREFIX}v7`, `${CACHE_PREFIX}v8`, `${CACHE_PREFIX}v9`]);
			expect(KEEP_UNANSWERED).toBe(2);
		} finally {
			vi.useRealTimers();
		}
	});

	test('a malformed answer counts as no answer (it cannot name a cache to keep or delete)', async () => {
		const w = world('v3', [page(() => ({ type: MSG.reply, version: '../../v1' })), page(() => ({ type: 'other', version: 'v1' })), page(() => 'v1')]);
		await withOld(w, 'v0', 'v1', 'v2');
		await onActivate(w.env);
		const left = (await w.caches.keys()).filter((k) => k.startsWith(CACHE_PREFIX));
		expect(left).toEqual([`${CACHE_PREFIX}v1`, `${CACHE_PREFIX}v2`, `${CACHE_PREFIX}v3`]);
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
		const res = await onFetch(w.env, req('_app/immutable/nodes/4.v2hash.js'))!;
		expect(await res.text()).toBe(`fresh ${SCOPE}_app/immutable/nodes/4.v2hash.js`);
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
