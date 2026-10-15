// Changing one's own password (Settings › Account): the request, the checks before
// sending, each refusal's message, password-manager hints and the devices left signed in.
import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import ChangePassword from '$lib/components/ChangePassword.svelte';
import { ApiError, callJson, retryAfterOf } from '$lib/api/http';
import { changePassword, features, type PasswordChange, type PasswordChanged } from '$lib/api/auth';
import { changeError, deviceDetail, formProblem, waitText } from '$lib/account/change-password';

const settle = async () => {
	for (let i = 0; i < 5; i++) await tick();
};

const OLD = 'correct horse battery';
const NEW = 'a much longer passphrase';

describe('the API call', () => {
	type Call = { url: string; method: string; headers: Record<string, string>; body: unknown };
	let calls: Call[] = [];
	let reply: { status: number; body: unknown; headers?: Record<string, string> } = { status: 200, body: {} };
	beforeEach(() => {
		calls = [];
		reply = { status: 200, body: { signed_out: 0, other_devices: [] } };
		globalThis.fetch = vi.fn(async (url: RequestInfo | URL, init?: RequestInit) => {
			calls.push({
				url: String(url),
				method: init?.method ?? 'GET',
				headers: init?.headers as Record<string, string>,
				body: init?.body ? JSON.parse(String(init.body)) : undefined,
			});
			return new Response(JSON.stringify(reply.body), { status: reply.status, headers: { 'content-type': 'application/json', ...reply.headers } });
		}) as typeof fetch;
	});

	test('sends exactly the fields the gateway takes, with the token and client header', async () => {
		await changePassword('tok', { current: OLD, new: NEW });
		expect(calls[0]).toMatchObject({ url: '/v1/account/password', method: 'POST', body: { current: OLD, new: NEW } });
		expect(calls[0]!.headers.Authorization).toBe('Bearer tok');
		expect(calls[0]!.headers['X-Anywhere-Client']).toBe('1');
		expect(Object.keys(calls[0]!.body as object).sort()).toEqual(['current', 'new']);
		await changePassword('tok', { current: OLD, new: NEW, code: '123456', signOutOthers: true });
		expect(calls[1]!.body).toEqual({ current: OLD, new: NEW, code: '123456', sign_out_others: true });
	});

	test('errors carry the status, the two-step flag and Retry-After', async () => {
		reply = { status: 429, body: { error: 'too many attempts; try again later' }, headers: { 'retry-after': '900' } };
		const e = await changePassword('tok', { current: OLD, new: NEW }).catch((x) => x);
		expect(e).toBeInstanceOf(ApiError);
		expect(e.status).toBe(429);
		expect(e.retryAfter).toBe(900);
		reply = { status: 403, body: { error: 'two-step code required', totp_required: true } };
		const t = await changePassword('tok', { current: OLD, new: NEW }).catch((x) => x);
		expect(t.status).toBe(403);
		expect(t.totpRequired).toBe(true);
		expect(t.retryAfter).toBeUndefined();
		reply = { status: 400, body: { error: 'password must be at least 10 characters' } };
		const w = (await callJson('/v1/account/password', { method: 'POST' }).catch((x) => x)) as ApiError;
		expect(w.message).toBe('password must be at least 10 characters');
	});

	test('Retry-After: whole seconds only, bounded', () => {
		expect(retryAfterOf('60')).toBe(60);
		expect(retryAfterOf(' 5 ')).toBe(5);
		expect(retryAfterOf('0')).toBe(1);
		expect(retryAfterOf('999999999')).toBe(86_400);
		for (const bad of [null, '', 'soon', '-1', '1.5', 'Wed, 21 Oct 2026 07:28:00 GMT', '1e9']) expect(retryAfterOf(bad)).toBeUndefined();
	});

	test('features: change_password only when the gateway says so', async () => {
		reply = { status: 200, body: { user: {}, features: { change_password: true } } };
		expect(await features('tok')).toEqual({ change_password: true, devices: false });
		expect(calls.at(-1)!.url).toBe('/v1/me');
		reply = { status: 200, body: { user: {}, features: { queue_edit: true } } };
		expect(await features('tok')).toEqual({ change_password: false, devices: false });
		reply = { status: 200, body: { user: {} } };
		expect(await features('tok')).toEqual({});
		reply = { status: 200, body: { features: { change_password: 'yes' } } };
		expect(await features('tok')).toEqual({ change_password: false, devices: false });
	});
});

describe('checks and messages', () => {
	const form = { current: OLD, next: NEW, confirm: NEW, totp: false, code: '' };

	test('the form is checked before sending', () => {
		expect(formProblem(form)).toBeNull();
		expect(formProblem({ ...form, current: '' })).toMatch(/current password/);
		expect(formProblem({ ...form, next: 'short', confirm: 'short' })).toMatch(/at least 10/);
		expect(formProblem({ ...form, next: 'x'.repeat(1025), confirm: 'x'.repeat(1025) })).toMatch(/too long/);
		expect(formProblem({ ...form, next: OLD, confirm: OLD })).toMatch(/different/);
		expect(formProblem({ ...form, confirm: NEW + '!' })).toMatch(/don’t match/);
		expect(formProblem({ ...form, totp: true })).toMatch(/6-digit code/);
		expect(formProblem({ ...form, totp: true, code: '12345a' })).toMatch(/6-digit code/);
		expect(formProblem({ ...form, totp: true, code: '123 456' })).toBeNull();
	});

	test('each refusal reads clearly', () => {
		expect(changeError(new ApiError(403, 'the current password or two-step code is wrong')).message).toBe('The current password or two-step code isn’t right.');
		const code = changeError(new ApiError(403, 'two-step code required', true));
		expect(code.needCode).toBe(true);
		expect(code.message).toMatch(/authenticator/);
		expect(changeError(new ApiError(400, 'password must be at least 10 characters')).message).toBe('Password must be at least 10 characters.');
		expect(changeError(new ApiError(400, 'the new password must be different from the current one')).message).toBe('The new password must be different from the current one.');
		expect(changeError(new ApiError(429, 'too many', false, 900)).message).toBe('Too many attempts. Try again in 15 minutes.');
		expect(changeError(new ApiError(429, 'too many')).message).toBe('Too many attempts. Try again later.');
		expect(changeError(new ApiError(401, 'invalid credentials')).message).toMatch(/session has ended/);
		expect(changeError(new ApiError(503, 'busy', false, 1)).message).toMatch(/busy/);
		expect(changeError(new TypeError('Failed to fetch')).message).toMatch(/Couldn’t reach the server/);
		expect(waitText(30)).toBe('in 1 minute');
		expect(waitText(4 * 3600)).toBe('in 4 hours');
	});

	test('devices read as platform and last seen', () => {
		const now = 1_800_000_000_000;
		expect(deviceDetail({ id: 1, name: 'Phone', client_platform: 'Pixel 8', last_seen: now / 1000 - 3 * 3600 }, now)).toBe('Pixel 8 · last seen 3 hours ago');
		expect(deviceDetail({ id: 2, name: 'Tab', client_kind: 'android' }, now)).toBe('Android app');
		expect(deviceDetail({ id: 3, name: 'Old' }, now)).toBe('');
	});
});

describe('the form', () => {
	const ok: PasswordChanged = { signed_out: 0, other_devices: [] };

	async function fill(c: HTMLElement, v: { current?: string; next?: string; confirm?: string; code?: string }) {
		const set = async (sel: string, value: string | undefined) => {
			if (value === undefined) return;
			await fireEvent.input(c.querySelector(sel)!, { target: { value } });
		};
		await set('[data-current]', v.current);
		await set('[data-new]', v.next);
		await set('[data-confirm]', v.confirm);
		await set('[data-code]', v.code);
		await fireEvent.submit(c.querySelector('form[data-change-password]')!);
		await settle();
	}

	test('password managers: autocomplete hints and the account name', () => {
		const { container } = render(ChangePassword, { username: 'alice', totp: false, change: async () => ok });
		const user = container.querySelector<HTMLInputElement>('input[autocomplete=username]')!;
		expect(user.value).toBe('alice');
		expect(user.hidden).toBe(true);
		expect(container.querySelector('[data-current]')!.getAttribute('autocomplete')).toBe('current-password');
		expect(container.querySelector('[data-new]')!.getAttribute('autocomplete')).toBe('new-password');
		expect(container.querySelector('[data-confirm]')!.getAttribute('autocomplete')).toBe('new-password');
		for (const input of container.querySelectorAll('input[type=password]')) expect(input.closest('label')!.textContent!.trim().length).toBeGreaterThan(0);
		// Messages are announced.
		expect(container.querySelector('[data-change-status]')!.getAttribute('aria-live')).toBe('polite');
		// Signing out the other devices is a choice, off by default.
		expect(container.querySelector<HTMLInputElement>('[data-sign-out-others]')!.checked).toBe(false);
	});

	test('the two-step code field appears only when two-step is on', () => {
		const off = render(ChangePassword, { username: 'alice', totp: false, change: async () => ok });
		expect(off.container.querySelector('[data-code]')).toBeNull();
		off.unmount();
		const on = render(ChangePassword, { username: 'alice', totp: true, change: async () => ok });
		expect(on.container.querySelector('[data-code]')!.getAttribute('autocomplete')).toBe('one-time-code');
		on.unmount();
	});

	test('a mismatched confirmation is not sent', async () => {
		const change = vi.fn(async () => ok);
		const { container } = render(ChangePassword, { username: 'alice', totp: false, change });
		await fill(container, { current: OLD, next: NEW, confirm: 'something else entirely' });
		expect(change).not.toHaveBeenCalled();
		expect(container.querySelector('[data-change-error]')!.textContent).toMatch(/don’t match/);
	});

	test('success: sends the change, clears the fields and lists the devices still signed in', async () => {
		const change = vi.fn(async (_c: PasswordChange): Promise<PasswordChanged> => ({
			signed_out: 0,
			other_devices: [{ id: 7, name: 'Kitchen phone', client_platform: 'Pixel 8', last_seen: null }],
		}));
		const { container } = render(ChangePassword, { username: 'alice', totp: true, change });
		await fill(container, { current: OLD, next: NEW, confirm: NEW, code: '123 456' });
		expect(change).toHaveBeenCalledWith({ current: OLD, new: NEW, code: '123456', signOutOthers: false });
		expect(container.querySelector('[data-change-done]')!.textContent).toMatch(/has been changed/);
		expect(container.querySelector('[data-other-devices]')!.textContent).toContain('Kitchen phone');
		expect(container.querySelector('[data-other-devices]')!.textContent).toContain('Pixel 8');
		for (const input of container.querySelectorAll<HTMLInputElement>('input[type=password], [data-code]')) expect(input.value).toBe('');
		// No password is ever shown.
		expect(container.textContent).not.toContain(NEW);
		expect(container.textContent).not.toContain(OLD);
	});

	test('signing out the others says how many went', async () => {
		const change = vi.fn(async (): Promise<PasswordChanged> => ({ signed_out: 2, other_devices: [] }));
		const { container } = render(ChangePassword, { username: 'alice', totp: false, change });
		await fireEvent.click(container.querySelector('[data-sign-out-others]')!);
		await fill(container, { current: OLD, next: NEW, confirm: NEW });
		expect(change).toHaveBeenCalledWith(expect.objectContaining({ signOutOthers: true }));
		expect(container.querySelector('[data-signed-out]')!.textContent).toBe('2 other devices were signed out.');
		expect(container.querySelector('[data-other-devices]')).toBeNull();
	});

	test('refusals show inline, and a code asked for by the server adds the field', async () => {
		for (const [err, want] of [
			[new ApiError(403, 'the current password or two-step code is wrong'), 'The current password or two-step code isn’t right.'],
			[new ApiError(400, 'password too long'), 'Password too long.'],
			[new ApiError(429, 'too many attempts; try again later', false, 120), 'Too many attempts. Try again in 2 minutes.'],
			[new TypeError('Failed to fetch'), 'Couldn’t reach the server. Check your connection and try again.'],
			[new ApiError(401, 'invalid credentials'), 'Your session has ended. Sign in again, then change your password.'],
		] as const) {
			const { container, unmount } = render(ChangePassword, { username: 'alice', totp: false, change: () => Promise.reject(err) });
			await fill(container, { current: OLD, next: NEW, confirm: NEW });
			expect(container.querySelector('[data-change-error]')!.textContent).toBe(want);
			expect(container.querySelector('[data-change-done]')).toBeNull();
			unmount();
		}
		const { container } = render(ChangePassword, { username: 'alice', totp: false, change: () => Promise.reject(new ApiError(403, 'two-step code required', true)) });
		expect(container.querySelector('[data-code]')).toBeNull();
		await fill(container, { current: OLD, next: NEW, confirm: NEW });
		expect(container.querySelector('[data-code]')).not.toBeNull();
		expect(container.querySelector('[data-change-error]')!.textContent).toMatch(/authenticator/);
	});
});
