// The boot guard (static/boot-guard.js): a page whose app never starts (a stale or broken
// stored shell) clears this app's service worker and stored shell and reloads, once per five
// minutes at most, never offline, never when the server does not answer.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { runInNewContext } from 'node:vm';

const SOURCE = readFileSync(resolve(process.cwd(), 'static/boot-guard.js'), 'utf8');

interface Guard {
	decide(s: { booted: boolean; online: boolean; now: number; last: number | null }): 'repair' | 'none';
	check(): Promise<boolean>;
	KEY: string;
	WAIT_MS: number;
	GAP_MS: number;
}

/** A fake window: storage, service worker registrations, caches, the server and the clock. */
function fakeWindow(o: { online?: boolean; booted?: boolean; readyState?: string; storage?: 'ok' | 'throws'; server?: 'ok' | 'error' | 'down'; noSw?: boolean } = {}) {
	const store = new Map<string, string>();
	const unregistered: number[] = [];
	const cacheNames = ['anywhere-shell-0.7.0', 'anywhere-shell-0.6.3', 'anywhere-downloads', 'other'];
	const fetched: string[] = [];
	const listeners: Record<string, () => void> = {};
	const attrs: Record<string, string> = o.booted ? { 'data-la-booted': '1' } : {};
	const throws = () => {
		throw new DOMException('denied', 'SecurityError');
	};
	const w = {
		localStorage: o.storage === 'throws' ? { getItem: throws, setItem: throws } : { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) },
		navigator: {
			onLine: o.online ?? true,
			serviceWorker: o.noSw ? undefined : { getRegistrations: async () => [1, 2].map((n) => ({ unregister: async () => void unregistered.push(n) })) },
		},
		caches: { keys: async () => [...cacheNames], delete: async (k: string) => (cacheNames.splice(cacheNames.indexOf(k), 1), true) },
		location: { reload: vi.fn() },
		document: { readyState: o.readyState ?? 'complete', documentElement: { getAttribute: (k: string) => attrs[k] ?? null } },
		addEventListener: (type: string, fn: () => void) => void (listeners[type] = fn),
		setTimeout: (fn: () => void, ms: number) => setTimeout(fn, ms),
		fetch: async (url: string) => {
			fetched.push(url);
			if (o.server === 'down') throw new TypeError('Failed to fetch');
			return { ok: o.server !== 'error' };
		},
		laBootGuard: undefined as unknown as Guard,
	};
	return { w, store, unregistered, cacheNames, fetched, listeners, attrs };
}

/** Run the guard against a fake window, as the browser would (with this test's clock). */
function load(f: ReturnType<typeof fakeWindow>): Guard {
	runInNewContext(SOURCE, { window: f.w, Date, Promise });
	return f.w.laBootGuard;
}

const T0 = Date.UTC(2026, 9, 24, 12);

beforeEach(() => {
	vi.useFakeTimers();
	vi.setSystemTime(T0);
});
afterEach(() => {
	vi.useRealTimers();
});

describe('decide', () => {
	const g = () => load(fakeWindow({ readyState: 'loading' }));
	const base = { booted: false, online: true, now: T0, last: null };
	test.each([
		[base, 'repair'],
		[{ ...base, booted: true }, 'none'],
		[{ ...base, online: false }, 'none'],
		[{ ...base, last: T0 - 1000 }, 'none'], // repaired a moment ago: no loop
		[{ ...base, last: T0 - 5 * 60 * 1000 + 1 }, 'none'],
		[{ ...base, last: T0 - 5 * 60 * 1000 }, 'repair'],
		[{ ...base, last: T0 + 60_000 }, 'repair'], // a record from the future (the clock moved): not trusted
	] as const)('%o → %s', (s, want) => {
		expect(g().decide(s)).toBe(want);
	});
});

describe('the guard in a page', () => {
	test('not started 10 s after load: unregisters the workers, clears only the shell caches, notes it, reloads', async () => {
		const f = fakeWindow();
		load(f);
		await vi.advanceTimersByTimeAsync(9_999);
		expect(f.w.location.reload).not.toHaveBeenCalled();
		await vi.advanceTimersByTimeAsync(1);
		expect(f.w.location.reload).toHaveBeenCalledOnce();
		expect(f.unregistered).toEqual([1, 2]);
		expect(f.cacheNames).toEqual(['anywhere-downloads', 'other']);
		expect(f.store.get('la-boot-guard')).toBe(String(T0 + 10_000));
		// The server was asked past the service worker (a query), without the stored copy.
		expect(f.fetched).toHaveLength(1);
		expect(f.fetched[0]).toMatch(/^\.\/\?la-boot-check=\d+$/);
	});

	test('waits for the load event when the page is still loading', async () => {
		const f = fakeWindow({ readyState: 'loading' });
		load(f);
		await vi.advanceTimersByTimeAsync(60_000);
		expect(f.w.location.reload).not.toHaveBeenCalled();
		f.listeners.load!();
		await vi.advanceTimersByTimeAsync(10_000);
		expect(f.w.location.reload).toHaveBeenCalledOnce();
	});

	test('a started app is left alone', async () => {
		const f = fakeWindow();
		load(f);
		f.attrs['data-la-booted'] = '1';
		await vi.advanceTimersByTimeAsync(20_000);
		expect(f.w.location.reload).not.toHaveBeenCalled();
		expect(f.unregistered).toEqual([]);
		expect(f.cacheNames).toHaveLength(4);
		expect(f.fetched).toEqual([]);
	});

	test('the app starting while the server is asked: left alone', async () => {
		const f = fakeWindow({ readyState: 'loading' });
		const g = load(f);
		const asked = g.check();
		f.attrs['data-la-booted'] = '1';
		expect(await asked).toBe(false);
		expect(f.w.location.reload).not.toHaveBeenCalled();
	});

	test('offline: nothing is touched (the stored shell is all there is)', async () => {
		const f = fakeWindow({ online: false });
		load(f);
		await vi.advanceTimersByTimeAsync(20_000);
		expect(f.w.location.reload).not.toHaveBeenCalled();
		expect(f.unregistered).toEqual([]);
		expect(f.fetched).toEqual([]);
	});

	test('the server down or answering an error: nothing is touched', async () => {
		for (const server of ['down', 'error'] as const) {
			const f = fakeWindow({ server, readyState: 'loading' });
			expect(await load(f).check()).toBe(false);
			expect(f.w.location.reload).not.toHaveBeenCalled();
			expect(f.unregistered).toEqual([]);
			expect(f.cacheNames).toHaveLength(4);
		}
	});

	test('a second broken load within five minutes does not reload again; after five minutes it may', async () => {
		const f = fakeWindow();
		load(f);
		await vi.advanceTimersByTimeAsync(10_000);
		expect(f.w.location.reload).toHaveBeenCalledOnce();
		// The reloaded page is broken too (same storage, new page).
		const again = fakeWindow();
		again.store.set('la-boot-guard', f.store.get('la-boot-guard')!);
		load(again);
		await vi.advanceTimersByTimeAsync(10_000);
		expect(again.w.location.reload).not.toHaveBeenCalled();
		expect(again.unregistered).toEqual([]);
		vi.setSystemTime(Date.now() + 5 * 60 * 1000);
		const later = fakeWindow();
		later.store.set('la-boot-guard', f.store.get('la-boot-guard')!);
		load(later);
		await vi.advanceTimersByTimeAsync(10_000);
		expect(later.w.location.reload).toHaveBeenCalledOnce();
	});

	test('a garbled record counts as none', async () => {
		const f = fakeWindow({ readyState: 'loading' });
		f.store.set('la-boot-guard', 'not a number');
		expect(await load(f).check()).toBe(true);
	});

	test('storage that throws: it clears, but does not reload (it could not guard against a loop)', async () => {
		const f = fakeWindow({ storage: 'throws' });
		load(f);
		await vi.advanceTimersByTimeAsync(10_000);
		expect(f.unregistered).toEqual([1, 2]);
		expect(f.cacheNames).toEqual(['anywhere-downloads', 'other']);
		expect(f.w.location.reload).not.toHaveBeenCalled();
	});

	test('no service worker support, or a failing one: the caches are still cleared and the page reloads', async () => {
		const f = fakeWindow({ noSw: true, readyState: 'loading' });
		expect(await load(f).check()).toBe(true);
		expect(f.cacheNames).toEqual(['anywhere-downloads', 'other']);
		const g = fakeWindow({ readyState: 'loading' });
		g.w.navigator.serviceWorker!.getRegistrations = async () => {
			throw new Error('denied');
		};
		g.w.caches.keys = async () => {
			throw new Error('denied');
		};
		expect(await load(g).check()).toBe(true);
		expect(g.w.location.reload).toHaveBeenCalledOnce();
	});
});

describe('wired into the page', () => {
	test('app.html loads it from a file, before SvelteKit’s head', () => {
		const html = readFileSync(resolve(process.cwd(), 'src/app.html'), 'utf8');
		const at = html.indexOf('<script src="./boot-guard.js"></script>');
		expect(at).toBeGreaterThan(0);
		expect(at).toBeLessThan(html.indexOf('%sveltekit.head%'));
	});

	test('the layout marks the app started', () => {
		const layout = readFileSync(resolve(process.cwd(), 'src/routes/+layout.svelte'), 'utf8');
		expect(layout).toContain("document.documentElement.dataset.laBooted = '1'");
	});

	test('a plain classic script, short: no modules, no eval', () => {
		expect(SOURCE).not.toMatch(/\b(import|export)\b/);
		expect(SOURCE).not.toMatch(/\beval\s*\(|\bFunction\s*\(/);
		expect(SOURCE.split('\n').length).toBeLessThanOrEqual(64);
	});
});
