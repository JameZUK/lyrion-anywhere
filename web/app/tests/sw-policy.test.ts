// Service worker: only the app's own files are ever stored; the API, audio and artwork never.
import { cacheable, strategyFor } from '$lib/pwa/sw-policy';

const scope = new URL('https://music.example.com/');
const sub = new URL('https://music.example.com/lms/');
const s = (path: string, method = 'GET', base = scope) => strategyFor(method, new URL(path, base), base);

test('the API, audio, artwork and player files are never handled', () => {
	for (const p of ['v1/rpc', 'v1/auth/refresh', 'v1/art/music/1/cover', 'v1/stream/abc', 'v1/events', 'v1', 'admin/v1/users', 'player/player-worker.js', 'player/pkg/anywhere_wasm_bg.wasm', 'player/silence.wav']) {
		expect(s(p), p).toBe('bypass');
	}
	expect(s('v1/rpc', 'GET', sub)).toBe('bypass');
});

// Offline downloads live in the app's own storage, never in the service worker's caches.
test('downloads are never handled by the service worker', () => {
	for (const p of ['v1/download/track/1/info', 'v1/download/track/1?format=original', 'v1/download/track/1']) {
		expect(s(p), p).toBe('bypass');
		expect(s(p, 'GET', sub), p).toBe('bypass');
	}
});

test('only GETs within the app, without queries', () => {
	expect(s('', 'POST')).toBe('bypass');
	expect(s('https://cdn.example.org/x.js')).toBe('bypass');
	expect(s('https://music.example.com/other/', 'GET', sub)).toBe('bypass');
	expect(s('?token=abc')).toBe('bypass');
	expect(s('_app/immutable/x.js?v=1')).toBe('bypass');
	expect(s('unknown/thing.json')).toBe('bypass');
});

test('the app shell is stored: hashed files kept, the rest refreshed', () => {
	expect(s('_app/immutable/entry/start.abc123.js')).toBe('cache-first');
	expect(s('')).toBe('network-first');
	expect(s('index.html')).toBe('network-first');
	expect(s('_app/version.json')).toBe('network-first');
	expect(s('manifest.webmanifest')).toBe('network-first');
	expect(s('icons/icon-192.png')).toBe('network-first');
	expect(s('favicon.svg')).toBe('network-first');
	expect(s('_app/immutable/a.js', 'GET', sub)).toBe('cache-first');
	expect(s('', 'GET', sub)).toBe('network-first');
});

test('only complete, ordinary answers are stored', () => {
	expect(cacheable(new Response('x', { status: 200 }))).toBe(false); // type "default", not "basic", outside a browser
	const basic = (status: number, headers: Record<string, string> = {}) => {
		const r = new Response(status === 204 ? null : 'x', { status, headers });
		Object.defineProperty(r, 'type', { value: 'basic' });
		return r;
	};
	expect(cacheable(basic(200))).toBe(true);
	expect(cacheable(basic(206))).toBe(false);
	expect(cacheable(basic(404))).toBe(false);
	expect(cacheable(basic(200, { 'cache-control': 'private, no-store' }))).toBe(false);
});

import { installWay } from '$lib/pwa/install.svelte';

test('install: prompt where offered, Share-menu steps on iPhone and iPad, nothing once installed', () => {
	const base = { standalone: false, hasPrompt: false, touchPoints: 0 };
	const android = 'Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Mobile Safari/537.36';
	const iphone = 'Mozilla/5.0 (iPhone; CPU iPhone OS 26_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Mobile/15E148 Safari/604.1';
	const ipad = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/27.0.1 Safari/605.1.15';
	expect(installWay({ ...base, userAgent: android, hasPrompt: true })).toBe('prompt');
	expect(installWay({ ...base, userAgent: android })).toBe('none');
	expect(installWay({ ...base, userAgent: iphone })).toBe('ios');
	expect(installWay({ ...base, userAgent: ipad, touchPoints: 5 })).toBe('ios');
	expect(installWay({ ...base, userAgent: ipad, touchPoints: 0 })).toBe('none'); // a real Mac
	expect(installWay({ ...base, userAgent: iphone, standalone: true })).toBe('installed');
});
