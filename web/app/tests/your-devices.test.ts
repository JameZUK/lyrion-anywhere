// "Your devices" (Settings › Account): the API calls, the checks on what the gateway sends,
// how each device reads, each refusal's message, and the list with its confirms.
import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import YourDevices from '$lib/components/YourDevices.svelte';
import { ApiError } from '$lib/api/http';
import { features, myDevices, signOutDevice, signOutOtherDevices, type MyDevice } from '$lib/api/auth';
import { MAX_DEVICES, ago, cleanDevices, deviceLine, devicesError } from '$lib/account/devices';

const settle = async () => {
	for (let i = 0; i < 6; i++) await tick();
};

describe('the API calls', () => {
	type Call = { url: string; method: string; headers: Record<string, string>; body: unknown };
	let calls: Call[] = [];
	let reply: { status: number; body: unknown } = { status: 200, body: {} };
	beforeEach(() => {
		calls = [];
		reply = { status: 200, body: {} };
		globalThis.fetch = vi.fn(async (url: RequestInfo | URL, init?: RequestInit) => {
			calls.push({ url: String(url), method: init?.method ?? 'GET', headers: init?.headers as Record<string, string>, body: init?.body });
			if (reply.status === 204) return new Response(null, { status: 204 });
			return new Response(JSON.stringify(reply.body), { status: reply.status, headers: { 'content-type': 'application/json' } });
		}) as typeof fetch;
	});

	test('list, sign out one, sign out the others: the token and client header on each', async () => {
		reply = { status: 200, body: { devices: [{ id: 1, name: 'Phone', current: true }] } };
		expect(await myDevices('tok')).toEqual([{ id: 1, name: 'Phone', current: true }]);
		expect(calls[0]).toMatchObject({ url: '/v1/devices', method: 'GET' });
		reply = { status: 204, body: null };
		await signOutDevice('tok', 42);
		expect(calls[1]).toMatchObject({ url: '/v1/devices/42', method: 'DELETE', body: undefined });
		reply = { status: 200, body: { signed_out: 3 } };
		expect(await signOutOtherDevices('tok')).toBe(3);
		expect(calls[2]).toMatchObject({ url: '/v1/devices/sign-out-others', method: 'POST', body: undefined });
		for (const c of calls) {
			expect(c.headers.Authorization).toBe('Bearer tok');
			expect(c.headers['X-Anywhere-Client']).toBe('1');
		}
	});

	test('only a plain positive whole id is ever put in the address', async () => {
		for (const bad of [0, -1, 1.5, NaN, Infinity, 2 ** 60]) {
			const e = await signOutDevice('tok', bad).catch((x) => x);
			expect(e).toBeInstanceOf(ApiError);
			expect(e.status).toBe(404);
		}
		// Even a string smuggled past the types.
		const e = await signOutDevice('tok', '1/../../admin' as unknown as number).catch((x) => x);
		expect(e.status).toBe(404);
		expect(calls).toHaveLength(0);
	});

	test('a nonsense count reads as none', async () => {
		for (const body of [{}, { signed_out: 'x' }, { signed_out: -1 }, { signed_out: 1.5 }]) {
			reply = { status: 200, body };
			expect(await signOutOtherDevices('tok')).toBe(0);
		}
	});

	test('refusals carry their status', async () => {
		reply = { status: 404, body: { error: 'no such device' } };
		expect((await signOutDevice('tok', 7).catch((x) => x)).status).toBe(404);
		reply = { status: 409, body: { error: 'this is the device you are using; sign out instead' } };
		expect((await signOutDevice('tok', 7).catch((x) => x)).status).toBe(409);
	});

	test('features: devices only when the gateway says so', async () => {
		reply = { status: 200, body: { features: { devices: true, change_password: true } } };
		expect(await features('tok')).toEqual({ change_password: true, devices: true });
		for (const f of [{ devices: 'yes' }, { devices: 1 }, {}]) {
			reply = { status: 200, body: { features: f } };
			expect((await features('tok')).devices).toBe(false);
		}
		reply = { status: 200, body: {} };
		expect((await features('tok')).devices).toBeUndefined();
	});
});

describe('checks and wording', () => {
	test('the list is checked: bad entries dropped, one current at most', () => {
		const list = cleanDevices([
			{ id: 1, name: 'Phone\u0007‮', client_kind: 'android', client_platform: 'Pixel 8', created_at: 1_600_000_000, last_seen: 1_700_000_040, current: true },
			{ id: 2, name: 'Kitchen', client_kind: 'toaster', client_platform: '  ', current: 'yes' },
			{ id: 3, name: 'Tablet', current: true },
			{ id: '4', name: 'string id' },
			{ id: 0, name: 'zero' },
			{ id: -5, name: 'negative' },
			{ id: 1.5, name: 'fraction' },
			{ id: 1, name: 'repeat' },
			{ id: 6 },
			{ id: 7, name: '​' },
			null,
			'nonsense',
		]);
		expect(list.map((d) => d.id)).toEqual([1, 2, 3]);
		expect(list[0]).toEqual({ id: 1, name: 'Phone', client_kind: 'android', client_platform: 'Pixel 8', created_at: 1_600_000_000, last_seen: 1_700_000_040, current: true });
		expect(list[1]).toMatchObject({ client_kind: null, client_platform: null, current: false, last_seen: null });
		expect(list[2]!.current).toBe(false);
		expect(cleanDevices([{ id: 9, name: 'x'.repeat(500), client_platform: 'y'.repeat(500) }])[0]).toMatchObject({ name: 'x'.repeat(64), client_platform: 'y'.repeat(48) });
		expect(cleanDevices(Array.from({ length: 500 }, (_, i) => ({ id: i + 1, name: 'd' })))).toHaveLength(MAX_DEVICES);
		for (const bad of [null, undefined, {}, 'x', 7]) expect(() => cleanDevices(bad)).toThrow(ApiError);
	});

	test('each device reads as what it is and when it was last used', () => {
		const now = 1_800_000_000_000;
		const d = (over: Partial<MyDevice>): MyDevice => ({ id: 2, name: 'x', client_kind: null, client_platform: null, created_at: null, last_seen: null, current: false, ...over });
		expect(deviceLine(d({ client_platform: 'Pixel 8', last_seen: now / 1000 - 3 * 3600 }), now)).toBe('Pixel 8 · Last used 3 hours ago');
		expect(deviceLine(d({ client_kind: 'android', last_seen: now / 1000 - 30 }), now)).toBe('Android app · Last used just now');
		expect(deviceLine(d({ client_kind: 'pwa' }), now)).toBe('Installed web app · Not used yet');
		expect(deviceLine(d({ client_kind: 'browser', current: true, last_seen: 1 }), now)).toBe('Web browser · In use now');
		// A clock behind the server's never reads as the future.
		expect(deviceLine(d({ last_seen: now / 1000 + 600 }), now)).toBe('Last used just now');
		expect(ago(5 * 60)).toBe('5 minutes ago');
		expect(ago(3 * 86_400)).toBe('3 days ago');
		expect(ago(120 * 86_400)).toBe('4 months ago');
	});

	test('each refusal reads clearly', () => {
		expect(devicesError(new ApiError(404, 'no such device'))).toMatch(/already signed out/);
		expect(devicesError(new ApiError(409, 'this is the device you are using'))).toMatch(/use Sign out/);
		expect(devicesError(new ApiError(401, 'invalid credentials'))).toMatch(/session has ended/);
		expect(devicesError(new ApiError(429, 'too many', false, 60))).toBe('Too many requests. Try again in 1 minute.');
		expect(devicesError(new ApiError(429, 'too many'))).toBe('Too many requests. Try again later.');
		expect(devicesError(new ApiError(503, 'busy'))).toMatch(/busy/);
		expect(devicesError(new ApiError(500, 'internal error'))).toMatch(/HTTP 500/);
		expect(devicesError(new TypeError('Failed to fetch'))).toMatch(/Couldn’t reach the server/);
		expect(devicesError(new Error('x'))).toMatch(/Something went wrong/);
	});
});

describe('the list', () => {
	const NOW = 1_800_000_000_000;
	const LIST = [
		{ id: 1, name: 'Laptop', client_kind: 'browser', client_platform: 'Firefox on Linux', created_at: 1, last_seen: NOW / 1000 - 60, current: true },
		{ id: 2, name: 'Phone', client_kind: 'android', client_platform: 'Pixel 8', created_at: 2, last_seen: NOW / 1000 - 2 * 3600, current: false },
		{ id: 3, name: 'Kitchen', client_kind: 'pwa', client_platform: null, created_at: 3, last_seen: null, current: false },
	];

	function setup(over: { list?: unknown[]; signOut?: (id: number) => Promise<void>; signOutOthers?: () => Promise<number> } = {}) {
		let list = over.list ?? LIST;
		const signedOut: number[] = [];
		const signOut = vi.fn(
			over.signOut ??
				(async (id: number) => {
					signedOut.push(id);
					list = list.filter((d) => (d as { id: number }).id !== id);
				}),
		);
		const signOutOthers = vi.fn(
			over.signOutOthers ??
				(async () => {
					const n = list.filter((d) => !(d as { current: boolean }).current).length;
					list = list.filter((d) => (d as { current: boolean }).current);
					return n;
				}),
		);
		const load = vi.fn(async () => list);
		const r = render(YourDevices, { load, signOut, signOutOthers, now: () => NOW });
		return { r, load, signOut, signOutOthers, signedOut };
	}

	async function confirm(yes: boolean) {
		const dialog = await waitFor(() => document.querySelector<HTMLDialogElement>('dialog[open][data-confirm-dialog]')!);
		await fireEvent.click(dialog.querySelector(yes ? '[data-confirm-ok]' : '[data-confirm-cancel]')!);
		await settle();
	}

	test('names, This device, last used; no sign-out on this device', async () => {
		const { r } = setup();
		await settle();
		const rows = [...r.container.querySelectorAll<HTMLElement>('[data-device]')];
		expect(rows.map((li) => li.querySelector('strong')!.textContent)).toEqual(['Laptop', 'Phone', 'Kitchen']);
		expect(rows[0]!.querySelector('[data-this-device]')!.textContent).toBe('This device');
		expect(rows[1]!.querySelector('[data-this-device]')).toBeNull();
		expect(rows[0]!.textContent).toContain('Firefox on Linux · In use now');
		expect(rows[1]!.textContent).toContain('Pixel 8 · Last used 2 hours ago');
		expect(rows[2]!.textContent).toContain('Installed web app · Not used yet');
		expect(rows[0]!.querySelector('[data-sign-out-device]')).toBeNull();
		expect(rows[1]!.querySelector('[data-sign-out-device]')!.getAttribute('aria-label')).toBe('Sign out Phone');
		expect(r.container.querySelector('[data-sign-out-others]')).not.toBeNull();
		expect(r.container.querySelector('[data-devices-status]')!.getAttribute('aria-live')).toBe('polite');
		r.unmount();
	});

	test('signing one out asks first; cancel does nothing, yes signs it out and refreshes', async () => {
		const { r, signOut, load } = setup();
		await settle();
		await fireEvent.click(r.container.querySelector('[data-sign-out-device="2"]')!);
		const dialog = await waitFor(() => document.querySelector<HTMLDialogElement>('dialog[open][data-confirm-dialog]')!);
		expect(dialog.textContent).toContain('Sign out “Phone”?');
		await confirm(false);
		expect(signOut).not.toHaveBeenCalled();
		await fireEvent.click(r.container.querySelector('[data-sign-out-device="2"]')!);
		await confirm(true);
		expect(signOut).toHaveBeenCalledWith(2);
		expect(r.container.querySelector('[data-devices-message]')!.textContent).toBe('“Phone” was signed out.');
		expect(load).toHaveBeenCalledTimes(2);
		expect(r.container.querySelector('[data-device="2"]')).toBeNull();
		r.unmount();
	});

	test('sign out all other devices asks first and keeps this one', async () => {
		const { r, signOutOthers } = setup();
		await settle();
		await fireEvent.click(r.container.querySelector('[data-sign-out-others]')!);
		const dialog = await waitFor(() => document.querySelector<HTMLDialogElement>('dialog[open][data-confirm-dialog]')!);
		expect(dialog.textContent).toContain('2 devices will have to sign in again. This device stays signed in.');
		await confirm(false);
		expect(signOutOthers).not.toHaveBeenCalled();
		await fireEvent.click(r.container.querySelector('[data-sign-out-others]')!);
		await confirm(true);
		expect(signOutOthers).toHaveBeenCalledTimes(1);
		expect(r.container.querySelector('[data-devices-message]')!.textContent).toBe('2 other devices were signed out.');
		expect([...r.container.querySelectorAll('[data-device]')]).toHaveLength(1);
		expect(r.container.querySelector('[data-sign-out-others]')).toBeNull();
		expect(r.container.querySelector('[data-no-other-devices]')).not.toBeNull();
		r.unmount();
	});

	test('one message at a time: an error replaces the last success', async () => {
		let fail = false;
		const { r } = setup({
			signOut: async () => {
				if (fail) throw new ApiError(429, 'too many', false, 60);
			},
		});
		await settle();
		await fireEvent.click(r.container.querySelector('[data-sign-out-device="2"]')!);
		await confirm(true);
		fail = true;
		await fireEvent.click(r.container.querySelector('[data-sign-out-device="3"]')!);
		await confirm(true);
		const msgs = r.container.querySelectorAll('[data-devices-message]');
		expect(msgs).toHaveLength(1);
		expect(msgs[0]!.textContent).toBe('Too many requests. Try again in 1 minute.');
		expect(msgs[0]!.classList.contains('error')).toBe(true);
		r.unmount();
	});

	test('a device already gone is said plainly, not as an error', async () => {
		const { r } = setup({
			signOut: async () => {
				throw new ApiError(404, 'no such device');
			},
		});
		await settle();
		await fireEvent.click(r.container.querySelector('[data-sign-out-device="3"]')!);
		await confirm(true);
		const msg = r.container.querySelector('[data-devices-message]')!;
		expect(msg.textContent).toMatch(/already signed out/);
		expect(msg.classList.contains('error')).toBe(false);
		r.unmount();
	});

	test('a failed load says why', async () => {
		const r = render(YourDevices, {
			load: async () => {
				throw new TypeError('Failed to fetch');
			},
			signOut: async () => {},
			signOutOthers: async () => 0,
		});
		await settle();
		expect(r.container.querySelector('[data-devices-message]')!.textContent).toMatch(/Couldn’t reach the server/);
		expect(r.container.querySelector('[data-device-list]')).toBeNull();
		r.unmount();
	});

	test('a failed load offers Retry, and Retry loads the list', async () => {
		let fail = true;
		const load = vi.fn(async () => {
			if (fail) throw new ApiError(503, 'busy');
			return LIST;
		});
		const r = render(YourDevices, { load, signOut: async () => {}, signOutOthers: async () => 0, now: () => NOW });
		await settle();
		expect(r.container.querySelector('[data-devices-message]')!.textContent).toMatch(/busy/);
		const retry = r.container.querySelector<HTMLButtonElement>('[data-devices-retry]');
		expect(retry).not.toBeNull();
		expect(retry!.textContent).toBe('Try again');
		fail = false;
		await fireEvent.click(retry!);
		await settle();
		expect(load).toHaveBeenCalledTimes(2);
		expect([...r.container.querySelectorAll('[data-device]')]).toHaveLength(3);
		expect(r.container.querySelector('[data-devices-retry]')).toBeNull();
		expect(r.container.querySelector('[data-devices-message]')).toBeNull();
		r.unmount();
	});

	test('a refresh that answers late never brings back a device signed out since', async () => {
		// The server's list at the moment each load is asked for; each answer is held until released.
		let server = LIST;
		const held: { snapshot: unknown[]; release: () => void }[] = [];
		let first = true;
		const load = vi.fn(() => {
			const snapshot = server;
			if (first) {
				first = false;
				return Promise.resolve(snapshot);
			}
			return new Promise<unknown>((res) => held.push({ snapshot, release: () => res(snapshot) }));
		});
		const signOut = vi.fn(async (id: number) => {
			server = server.filter((d) => d.id !== id);
		});
		const r = render(YourDevices, { load, signOut, signOutOthers: async () => 0, now: () => NOW });
		await settle();
		await fireEvent.click(r.container.querySelector('[data-sign-out-device="2"]')!);
		await confirm(true);
		expect(held).toHaveLength(1); // the refresh after signing out Phone is in flight
		// A second sign-out while it is in flight (the button may be off, as it should be).
		await fireEvent.click(r.container.querySelector('[data-sign-out-device="3"]')!);
		await settle();
		if (document.querySelector('dialog[open][data-confirm-dialog]')) await confirm(true);
		// The answers come back newest first.
		for (const h of [...held].reverse()) {
			h.release();
			await settle();
		}
		if (signOut.mock.calls.some(([id]) => id === 3)) {
			expect(r.container.querySelector('[data-device="3"]')).toBeNull();
		} else {
			// It waited: now it can go.
			held.length = 0;
			await fireEvent.click(r.container.querySelector('[data-sign-out-device="3"]')!);
			await confirm(true);
			for (const h of held) h.release();
			await settle();
		}
		expect([...r.container.querySelectorAll<HTMLElement>('[data-device]')].map((li) => li.dataset.device)).toEqual(['1']);
		expect(signOut.mock.calls.map(([id]) => id)).toEqual([2, 3]);
		r.unmount();
	});

	test('a sign-out that worked keeps its message, and the device goes, even if the refresh fails', async () => {
		let n = 0;
		const load = vi.fn(async () => {
			if (n++ > 0) throw new ApiError(503, 'busy');
			return LIST;
		});
		const r = render(YourDevices, { load, signOut: async () => {}, signOutOthers: async () => 0, now: () => NOW });
		await settle();
		await fireEvent.click(r.container.querySelector('[data-sign-out-device="2"]')!);
		await confirm(true);
		expect(load).toHaveBeenCalledTimes(2);
		const msg = r.container.querySelector('[data-devices-message]')!;
		expect(msg.textContent).toBe('“Phone” was signed out.');
		expect(msg.classList.contains('error')).toBe(false);
		expect(r.container.querySelector('[data-device="2"]')).toBeNull();
		expect(r.container.querySelector('[data-device="3"]')).not.toBeNull();
		r.unmount();
	});

	test('signing out all others clears them from the list even if the refresh fails', async () => {
		let n = 0;
		const load = vi.fn(async () => {
			if (n++ > 0) throw new TypeError('Failed to fetch');
			return LIST;
		});
		const r = render(YourDevices, { load, signOut: async () => {}, signOutOthers: async () => 2, now: () => NOW });
		await settle();
		await fireEvent.click(r.container.querySelector('[data-sign-out-others]')!);
		await confirm(true);
		expect(r.container.querySelector('[data-devices-message]')!.textContent).toBe('2 other devices were signed out.');
		expect([...r.container.querySelectorAll<HTMLElement>('[data-device]')].map((li) => li.dataset.device)).toEqual(['1']);
		r.unmount();
	});

	test('a 409 (this device) reaching the list is an error and signs nothing out', async () => {
		const { r, signOutOthers } = setup({
			signOut: async () => {
				throw new ApiError(409, 'this is the device you are using; sign out instead');
			},
		});
		await settle();
		await fireEvent.click(r.container.querySelector('[data-sign-out-device="2"]')!);
		await confirm(true);
		const msg = r.container.querySelector('[data-devices-message]')!;
		expect(msg.textContent).toMatch(/use Sign out/);
		expect(msg.classList.contains('error')).toBe(true);
		expect(signOutOthers).not.toHaveBeenCalled();
		expect([...r.container.querySelectorAll('[data-device]')]).toHaveLength(3);
		r.unmount();
	});

	test('a session that ends mid-operation says so', async () => {
		const { r } = setup({
			signOut: async () => {
				throw new ApiError(401, 'invalid credentials');
			},
		});
		await settle();
		await fireEvent.click(r.container.querySelector('[data-sign-out-device="3"]')!);
		await confirm(true);
		const msg = r.container.querySelector('[data-devices-message]')!;
		expect(msg.textContent).toBe('Your session has ended. Sign in again.');
		expect(msg.classList.contains('error')).toBe(true);
		r.unmount();
	});

	test('confirming twice signs the device out once', async () => {
		const { r, signOut } = setup();
		await settle();
		await fireEvent.click(r.container.querySelector('[data-sign-out-device="2"]')!);
		const dialog = await waitFor(() => document.querySelector<HTMLDialogElement>('dialog[open][data-confirm-dialog]')!);
		const ok = dialog.querySelector<HTMLButtonElement>('[data-confirm-ok]')!;
		await fireEvent.click(ok);
		await fireEvent.click(ok);
		await settle();
		expect(signOut).toHaveBeenCalledTimes(1);
		r.unmount();
	});

	test('an empty list says no other devices are signed in', async () => {
		const { r } = setup({ list: [] });
		await settle();
		expect(r.container.querySelectorAll('[data-device]')).toHaveLength(0);
		expect(r.container.querySelector('[data-no-other-devices]')).not.toBeNull();
		expect(r.container.querySelector('[data-sign-out-others]')).toBeNull();
		expect(r.container.querySelector('[data-devices-retry]')).toBeNull();
		r.unmount();
	});

	test('hostile names stay text', async () => {
		const evil = '<img src=x onerror="window.__pwned=1">';
		const { r } = setup({ list: [{ id: 1, name: 'Me', current: true }, { id: 5, name: evil, client_platform: '<script>alert(1)</script>' }] });
		await settle();
		expect(r.container.querySelector('img')).toBeNull();
		expect(r.container.querySelector('script')).toBeNull();
		expect(r.container.querySelector('[data-device="5"] strong')!.textContent).toBe(evil);
		await fireEvent.click(r.container.querySelector('[data-sign-out-device="5"]')!);
		const dialog = await waitFor(() => document.querySelector<HTMLDialogElement>('dialog[open][data-confirm-dialog]')!);
		expect(dialog.querySelector('img')).toBeNull();
		await confirm(false);
		expect((window as unknown as { __pwned?: number }).__pwned).toBeUndefined();
		r.unmount();
	});
});
