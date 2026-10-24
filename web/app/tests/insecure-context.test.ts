// Plain http:// to anything but loopback (a trusted network, SPEC D28) is an insecure context:
// browsers withhold the service worker, Cache API, navigator.storage, Web Locks,
// crypto.randomUUID, crypto.subtle, the wake lock and AudioWorklet. The app must not throw
// without them, and says once per session that the connection is unencrypted.
import { fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import InsecureNotice from '#lib/components/InsecureNotice.svelte';
import { INSECURE_DISMISSED_KEY, INSECURE_TEXT, InsecureNotice as Notice, insecureContextNotice } from '#lib/pwa/insecure.svelte';
import { AppUpdate } from '#lib/pwa/update.svelte';
import { requestPersist, storageEstimate, storageInfo } from '#lib/offline/settings';
import { newUuid } from '#lib/offline/plays.svelte';
import { withRefreshLock } from '#lib/api/auth';

const UUID_V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

class Mem {
	m = new Map<string, string>();
	getItem(k: string) {
		return this.m.get(k) ?? null;
	}
	setItem(k: string, v: string) {
		this.m.set(k, v);
	}
}

describe('the unencrypted-connection notice', () => {
	const lan = { protocol: 'http:', hostname: '10.0.0.10', isSecureContext: false };

	test('shown for plain http:// to a LAN or VPN address, or a name', () => {
		expect(insecureContextNotice(lan)).toBe(INSECURE_TEXT);
		expect(insecureContextNotice({ ...lan, hostname: '[fd00::10]' })).toBe(INSECURE_TEXT);
		expect(insecureContextNotice({ ...lan, hostname: 'lms.example.lan' })).toBe(INSECURE_TEXT);
		// A browser that doesn't report isSecureContext: still shown.
		expect(insecureContextNotice({ protocol: 'http:', hostname: '10.0.0.10' })).toBe(INSECURE_TEXT);
		expect(INSECURE_TEXT).toBe('Unencrypted connection. Installing the app, offline use and some player features need HTTPS.');
	});

	test('not shown for https://, loopback, file: or an opaque origin', () => {
		expect(insecureContextNotice({ protocol: 'https:', hostname: 'music.example.com', isSecureContext: true })).toBeNull();
		expect(insecureContextNotice({ protocol: 'https:', hostname: '10.0.0.10', isSecureContext: true })).toBeNull();
		for (const hostname of ['localhost', '127.0.0.1', '127.1.2.3', '[::1]']) {
			expect(insecureContextNotice({ protocol: 'http:', hostname, isSecureContext: true }), hostname).toBeNull();
			// Loopback is never "unencrypted on the network", whatever the browser says.
			expect(insecureContextNotice({ protocol: 'http:', hostname, isSecureContext: false }), hostname).toBeNull();
		}
		expect(insecureContextNotice({ protocol: 'file:', hostname: '', isSecureContext: false })).toBeNull();
		expect(insecureContextNotice({ protocol: 'file:', hostname: '10.0.0.10', isSecureContext: false })).toBeNull();
		// An opaque ("null") origin, e.g. a sandboxed frame or data: URL.
		expect(insecureContextNotice({ protocol: 'data:', hostname: '', isSecureContext: false })).toBeNull();
		expect(insecureContextNotice({ protocol: 'http:', hostname: '', isSecureContext: false })).toBeNull();
		expect(insecureContextNotice({})).toBeNull();
		// The browser was told to treat this origin as secure (a developer flag): no notice.
		expect(insecureContextNotice({ ...lan, isSecureContext: true })).toBeNull();
		// Case and look-alikes are not loopback.
		expect(insecureContextNotice({ ...lan, hostname: 'localhost.example.com' })).toBe(INSECURE_TEXT);
	});

	test('dismissed for the session, and remembered across a reload of the page', () => {
		const s = new Mem();
		const a = new Notice(lan, s);
		expect(a.shown).toBe(true);
		a.dismiss();
		expect(a.shown).toBe(false);
		expect(s.getItem(INSECURE_DISMISSED_KEY)).toBe('1');
		expect(new Notice(lan, s).shown).toBe(false);
		// Nothing to show on https:// at all.
		expect(new Notice({ protocol: 'https:', hostname: 'music.example.com', isSecureContext: true }, new Mem()).shown).toBe(false);
	});

	test('without sessionStorage (blocked, or throwing): dismissed for this page load only', () => {
		const throwing = {
			getItem: () => {
				throw new Error('denied');
			},
			setItem: () => {
				throw new Error('denied');
			},
		};
		for (const storage of [null, throwing]) {
			const n = new Notice(lan, storage);
			expect(n.shown).toBe(true);
			expect(() => n.dismiss()).not.toThrow();
			expect(n.shown).toBe(false);
			expect(new Notice(lan, storage).shown).toBe(true);
		}
		// A stored value other than '1' is not a dismissal.
		const odd = new Mem();
		odd.setItem(INSECURE_DISMISSED_KEY, 'yes');
		expect(new Notice(lan, odd).shown).toBe(true);
	});

	test('the component shows the text with a Dismiss button, and goes on Dismiss', async () => {
		const notice = new Notice(lan, new Mem());
		const r = render(InsecureNotice, { notice });
		const box = r.container.querySelector('[data-insecure-notice]')!;
		expect(box.getAttribute('role')).toBe('status');
		expect(box.textContent).toContain(INSECURE_TEXT);
		await fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }));
		await tick();
		expect(r.container.querySelector('[data-insecure-notice]')).toBeNull();
		r.unmount();
	});

	test('the component shows nothing on a secure page', () => {
		const r = render(InsecureNotice, { notice: new Notice({ protocol: 'https:', hostname: 'music.example.com', isSecureContext: true }, new Mem()) });
		expect(r.container.querySelector('[data-insecure-notice]')).toBeNull();
		r.unmount();
	});

	test('the app-wide notice reads the page’s own origin (jsdom: loopback, so none)', async () => {
		const { insecureNotice } = await import('#lib/pwa/insecure.svelte');
		expect(location.hostname).toBe('localhost');
		expect(insecureNotice.shown).toBe(false);
	});
});

describe('APIs withheld from insecure contexts', () => {
	afterEach(() => vi.unstubAllGlobals());

	test('crypto.randomUUID missing: a version-4 UUID from getRandomValues', () => {
		const getRandomValues = <T extends ArrayBufferView | null>(a: T) => globalThis.crypto.getRandomValues(a as never) as T;
		const seen = new Set<string>();
		for (let i = 0; i < 200; i++) {
			const u = newUuid({ getRandomValues } as never);
			expect(u).toMatch(UUID_V4);
			seen.add(u);
		}
		expect(seen.size).toBe(200);
		// The bits that make it version 4, variant 1, whatever the random bytes.
		expect(newUuid({ getRandomValues: <T>(a: T) => ((a as Uint8Array).fill(0xff), a) } as never)).toBe('ffffffff-ffff-4fff-bfff-ffffffffffff');
		expect(newUuid({ getRandomValues: <T>(a: T) => ((a as Uint8Array).fill(0), a) } as never)).toBe('00000000-0000-4000-8000-000000000000');
		// randomUUID is used where there is one.
		expect(newUuid({ randomUUID: () => '11111111-1111-4111-8111-111111111111', getRandomValues } as never)).toBe('11111111-1111-4111-8111-111111111111');
		// The default reads the global crypto.
		expect(newUuid()).toMatch(UUID_V4);
	});

	test('navigator.storage missing: estimate, info and persist answer without throwing', async () => {
		expect(await storageEstimate(undefined)).toEqual({});
		expect(await storageEstimate({})).toEqual({});
		expect(await storageEstimate({ estimate: () => Promise.reject(new Error('denied')) })).toEqual({});
		expect(await storageEstimate({ estimate: async () => ({ usage: 1, quota: 2 }) })).toEqual({ usage: 1, quota: 2 });
		expect(await storageInfo(undefined)).toEqual({});
		expect(await requestPersist(undefined)).toBe(false);
		expect(await requestPersist({ persisted: () => Promise.reject(new Error('x')) })).toBe(false);
		// The defaults, with the browser's navigator.storage gone.
		vi.stubGlobal('navigator', { userAgent: 'Test' });
		expect(await storageEstimate()).toEqual({});
		expect(await storageInfo()).toEqual({});
		expect(await requestPersist()).toBe(false);
	});

	test('navigator.serviceWorker missing: the update flow does nothing, offers nothing, throws nothing', () => {
		vi.stubGlobal('navigator', { userAgent: 'Test' });
		const u = new AppUpdate();
		expect(() => u.start('v1')).not.toThrow();
		expect(u.offered).toBe(false);
		expect(() => u.check()).not.toThrow();
		expect(() => u.dismiss()).not.toThrow();
		expect(u.offered).toBe(false);
	});

	test('navigator.locks missing: the refresh runs once, unlocked', async () => {
		vi.stubGlobal('navigator', { userAgent: 'Test' });
		const fn = vi.fn(async () => 'ok');
		expect(await withRefreshLock(fn)).toBe('ok');
		expect(fn).toHaveBeenCalledOnce();
	});
});
