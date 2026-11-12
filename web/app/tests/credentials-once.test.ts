// Review M10: a mistyped password in two-step on/off and "Add another device" was sent twice.
// The gateway answered it 401, like an expired token, so the session renewed and sent the same
// guess again: each typo counted twice towards the lockout and rotated the refresh token. Now
// a wrong password or code is 403 (as for a password change), and a call carrying one is
// never sent again by itself after a renewal. These run against a fake gateway.
import { fireEvent, render, waitFor, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { ApiError } from '#lib/api/http';
import { RenewedFirst, SessionManager, type AuthApi } from '#lib/api/session';
import type { Session } from '#lib/api/auth';
import { credentialsError } from '#lib/account/credentials';
import { changeError } from '#lib/account/change-password';

const PASSWORD = 'correct horse battery';
const sess = (token: string): Session => ({ access_token: token, expires_in: 900, user: { username: 'demo', role: 'admin' }, device: { id: 1, name: 'Test browser' } });

vi.mock('#lib/state/app.svelte', async () => {
	const fake = await import('./helpers/fake-app.svelte');
	const { SessionManager } = await import('#lib/api/session');
	let n = 0;
	const auth = {
		refreshes: 0,
		login: async () => sess('t0'),
		refresh: async () => {
			auth.refreshes++;
			return sess(`t${++n}`);
		},
		logout: async () => {},
	};
	return { ...fake, session: new SessionManager(auth, undefined), gatewayAuth: auth };
});
const mod = (await import('#lib/state/app.svelte')) as unknown as { session: SessionManager; gatewayAuth: { refreshes: number } };
const { default: AccountSettings } = await import('#lib/settings/AccountSettings.svelte');
const { default: AddDevice } = await import('#lib/components/AddDevice.svelte');
const { default: ChangePassword } = await import('#lib/components/ChangePassword.svelte');
const auth = await import('#lib/api/auth');

/**
 * The fake gateway. Tokens issued by the session are good unless `expired` names them; a wrong
 * password (or code) is answered 403, as the gateway does now. Every POST is recorded.
 */
const gw = {
	expired: new Set<string>(),
	totpOn: false,
	posts: [] as { path: string; token: string; body: Record<string, unknown> }[],
};
function json(status: number, body: unknown) {
	return new Response(status === 204 ? null : JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}
beforeAll(() => {
	vi.stubGlobal('fetch', async (path: string, init: RequestInit = {}) => {
		const p = String(path);
		const token = String((init.headers as Record<string, string> | undefined)?.Authorization ?? '').replace(/^Bearer /, '');
		const body = init.body ? JSON.parse(String(init.body)) : {};
		if (init.method === 'POST') gw.posts.push({ path: p, token, body });
		if (gw.expired.has(token)) return json(401, { error: 'invalid credentials' });
		if (p.endsWith('/v1/account')) return json(200, { username: 'demo', role: 'admin', totp_enabled: gw.totpOn });
		if (p.endsWith('/v1/me')) return json(200, { features: { change_password: true } });
		const right = body.password === PASSWORD || body.current === PASSWORD;
		const codeOk = body.code === undefined || body.code === '123456';
		if (p.endsWith('/v1/account/totp/setup')) return right ? json(200, { secret: 'JBSWY3DPEHPK3PXP', uri: 'otpauth://totp/x', qr_svg: '<svg/>' }) : json(403, { error: 'the password is wrong' });
		if (p.endsWith('/v1/account/totp/disable')) return right && codeOk ? json(204, null) : json(403, { error: 'the password or two-step code is wrong' });
		if (p.endsWith('/v1/devices/pair')) return right ? json(200, { code: 'ABCDEFGHJK', expires_in: 600 }) : json(403, { error: 'the password is wrong' });
		if (p.endsWith('/v1/account/password')) return right && codeOk ? json(200, { signed_out: 0, other_devices: [] }) : json(403, { error: 'the current password or two-step code is wrong' });
		return json(404, { error: 'not found' });
	});
});
afterAll(() => vi.unstubAllGlobals());
beforeEach(async () => {
	gw.expired.clear();
	gw.posts.length = 0;
	gw.totpOn = false;
	if (mod.session.status !== 'signed-in') await mod.session.restore();
	mod.gatewayAuth.refreshes = 0;
});

const settle = async () => {
	for (let i = 0; i < 6; i++) {
		await tick();
		await new Promise((r) => setTimeout(r, 0));
	}
};
const postsTo = (suffix: string) => gw.posts.filter((p) => p.path.endsWith(suffix));

describe('a call carrying a password is sent once per tap', () => {
	test('a wrong password (403) is sent once, and the token is not renewed', async () => {
		const call = (t: string) => auth.totpSetup(t, 'wrong password!');
		await expect(mod.session.authedOnce(call)).rejects.toMatchObject({ status: 403 });
		expect(postsTo('/totp/setup')).toHaveLength(1);
		expect(mod.gatewayAuth.refreshes).toBe(0);
	});

	test('an expired token (401): renewed for the next try, but the password is not sent again', async () => {
		gw.expired.add(mod.session.token);
		const call = (t: string) => auth.totpSetup(t, PASSWORD);
		await expect(mod.session.authedOnce(call)).rejects.toBeInstanceOf(RenewedFirst);
		expect(postsTo('/totp/setup')).toHaveLength(1);
		expect(mod.gatewayAuth.refreshes).toBe(1);
		// The next tap goes with the new token, once.
		expect(await mod.session.authedOnce(call)).toMatchObject({ secret: 'JBSWY3DPEHPK3PXP' });
		expect(postsTo('/totp/setup')).toHaveLength(2);
		expect(postsTo('/totp/setup').map((p) => p.token)).toEqual([expect.any(String), mod.session.token]);
	});

	test('the ordinary authed call is still retried after a renewal (no credentials in it)', async () => {
		gw.expired.add(mod.session.token);
		expect(await mod.session.authed((t) => auth.account(t))).toMatchObject({ username: 'demo' });
		expect(mod.gatewayAuth.refreshes).toBe(1);
	});
});

describe('the session alone', () => {
	const refusal = () => new ApiError(401, 'invalid credentials', false, undefined, { error: 'invalid credentials' });
	function fakeAuth(answers: (Session | Error)[]): AuthApi {
		return {
			login: async () => sess('t-login'),
			refresh: vi.fn(async () => {
				const a = answers.shift();
				if (!a) throw refusal();
				if (a instanceof Error) throw a;
				return a;
			}),
			logout: async () => {},
		};
	}

	test('a renewal the server refuses ends the sign-in; the 401 is what the form sees', async () => {
		const s = new SessionManager(fakeAuth([sess('a1')]), undefined);
		await s.restore();
		const fn = vi.fn(async () => Promise.reject(refusal()));
		await expect(s.authedOnce(fn)).rejects.toMatchObject({ status: 401 });
		expect(fn).toHaveBeenCalledTimes(1);
		expect(s.status).toBe('signed-out');
	});

	test('a renewal that cannot reach the server says so, and nothing is sent again', async () => {
		const s = new SessionManager(fakeAuth([sess('a1'), new TypeError('Failed to fetch')]), undefined);
		await s.restore();
		const fn = vi.fn(async () => Promise.reject(refusal()));
		await expect(s.authedOnce(fn)).rejects.toBeInstanceOf(TypeError);
		expect(fn).toHaveBeenCalledTimes(1);
		expect(s.status).toBe('signed-in');
	});

	test('other failures pass through untouched; before sign-in nothing is sent', async () => {
		const s = new SessionManager(fakeAuth([sess('a1')]), undefined);
		const fn = vi.fn(async () => 'x');
		await expect(s.authedOnce(fn)).rejects.toMatchObject({ status: 401 });
		expect(fn).not.toHaveBeenCalled();
		await s.restore();
		await expect(s.authedOnce(async () => Promise.reject(new ApiError(429, 'slow down')))).rejects.toMatchObject({ status: 429 });
		await expect(s.authedOnce(async () => Promise.reject(new TypeError('Failed to fetch')))).rejects.toBeInstanceOf(TypeError);
		expect(await s.authedOnce(async (t) => t)).toBe('a1');
	});
});

describe('what each refusal says', () => {
	test('403 is the wrong password, never "your session ended"; a renewal says to try again', () => {
		expect(credentialsError(new ApiError(403, 'the password is wrong'), 'Password not right.')).toBe('Password not right.');
		expect(credentialsError(new RenewedFirst(), 'Password not right.')).toMatch(/renewed first.*Try again/);
		expect(credentialsError(new ApiError(429, 'x', false, 120), 'Password not right.')).toBe('Too many attempts. Try again in 2 minutes.');
		expect(credentialsError(new ApiError(429, 'x'), 'Password not right.', 'Too many codes.')).toBe('Too many codes.');
		expect(credentialsError(new ApiError(401, 'invalid credentials'), 'Password not right.')).not.toBe('Password not right.');
		expect(credentialsError(new TypeError('Failed to fetch'), 'Password not right.')).toMatch(/reach the server/);
		expect(credentialsError(new Error('The server sent no usable code.'), 'Password not right.')).toBe('The server sent no usable code.');
		expect(credentialsError('junk', 'Password not right.')).toBe('Something went wrong. Try again.');
		// The password change form says the same about a renewal.
		expect(changeError(new RenewedFirst()).message).toMatch(/renewed first.*Try again/);
	});
});

describe('the forms against the gateway', () => {
	test('Two-step: a wrong password reads as such, is sent once, and renews nothing', async () => {
		const r = render(AccountSettings);
		await waitFor(() => expect(r.container.querySelector('[data-account-row="two-step"]')!.textContent).toContain('Off'));
		await fireEvent.click(r.container.querySelector('[data-account-row="two-step"]')!);
		const dialog = (await waitFor(() => document.querySelector<HTMLElement>('dialog[open] [data-two-step]')!)) as HTMLElement;
		await fireEvent.input(within(dialog).getByLabelText('Password'), { target: { value: 'wrong password!' } });
		await fireEvent.submit(dialog.querySelector('form')!);
		await waitFor(() => expect(dialog.querySelector('[data-two-step-status]')!.textContent).toBe('Password not right.'));
		expect(postsTo('/totp/setup')).toHaveLength(1);
		expect(mod.gatewayAuth.refreshes).toBe(0);
		// An expired token: the right password is sent once; the form asks for another try.
		gw.expired.add(mod.session.token);
		await fireEvent.input(within(dialog).getByLabelText('Password'), { target: { value: PASSWORD } });
		await fireEvent.submit(dialog.querySelector('form')!);
		await waitFor(() => expect(dialog.querySelector('[data-two-step-status]')!.textContent).toMatch(/renewed first/));
		expect(postsTo('/totp/setup')).toHaveLength(2);
		expect(mod.gatewayAuth.refreshes).toBe(1);
		await fireEvent.input(within(dialog).getByLabelText('Password'), { target: { value: PASSWORD } });
		await fireEvent.submit(dialog.querySelector('form')!);
		await waitFor(() => expect(dialog.querySelector('code.key')).not.toBeNull());
		expect(postsTo('/totp/setup')).toHaveLength(3);
		document.querySelector<HTMLDialogElement>('dialog[open]')!.close();
		r.unmount();
	});

	test('Two-step off: a wrong password or code reads as such and is sent once', async () => {
		gw.totpOn = true;
		const r = render(AccountSettings);
		await waitFor(() => expect(r.container.querySelector('[data-account-row="two-step"]')!.textContent).toContain('On'));
		await fireEvent.click(r.container.querySelector('[data-account-row="two-step"]')!);
		const dialog = (await waitFor(() => document.querySelector<HTMLElement>('dialog[open] [data-two-step]')!)) as HTMLElement;
		await fireEvent.input(within(dialog).getByLabelText('Password'), { target: { value: PASSWORD } });
		await fireEvent.input(within(dialog).getByLabelText('Authenticator code'), { target: { value: '000000' } });
		await fireEvent.submit(dialog.querySelector('form')!);
		await waitFor(() => expect(dialog.querySelector('[data-two-step-status]')!.textContent).toBe('Password or code not right.'));
		expect(postsTo('/totp/disable')).toHaveLength(1);
		expect(mod.gatewayAuth.refreshes).toBe(0);
		document.querySelector<HTMLDialogElement>('dialog[open]')!.close();
		r.unmount();
	});

	test('Add another device: a wrong password reads as such and is sent once; the password is never kept', async () => {
		const r = render(AddDevice, { create: (pw: string) => mod.session.authedOnce((t) => auth.createPairingCode(t, pw)), server: 'https://music.example.com' });
		const input = r.container.querySelector('input[type=password]') as HTMLInputElement;
		await fireEvent.input(input, { target: { value: 'wrong password!' } });
		await fireEvent.submit(r.container.querySelector('form[data-add-device]')!);
		await settle();
		expect(r.container.querySelector('[data-add-device-error]')!.textContent).toBe('Password not right.');
		expect(postsTo('/devices/pair')).toHaveLength(1);
		expect(mod.gatewayAuth.refreshes).toBe(0);
		expect(input.value).toBe('');
		gw.expired.add(mod.session.token);
		await fireEvent.input(input, { target: { value: PASSWORD } });
		await fireEvent.submit(r.container.querySelector('form[data-add-device]')!);
		await settle();
		expect(r.container.querySelector('[data-add-device-error]')!.textContent).toMatch(/renewed first/);
		expect(postsTo('/devices/pair')).toHaveLength(2);
		r.unmount();
	});

	test('Settings › Account sends the pairing password through the once-only call', async () => {
		const r = render(AccountSettings);
		await fireEvent.click(r.container.querySelector('[data-account-row="device"]')!);
		const dialog = (await waitFor(() => document.querySelector<HTMLElement>('dialog[open] [data-add-device]')!)) as HTMLElement;
		gw.expired.add(mod.session.token);
		await fireEvent.input(dialog.querySelector('input[type=password]')!, { target: { value: PASSWORD } });
		await fireEvent.submit(dialog);
		await waitFor(() => expect(document.querySelector('[data-add-device-error]')?.textContent ?? '').toMatch(/renewed first/));
		expect(postsTo('/devices/pair')).toHaveLength(1);
		document.querySelector<HTMLDialogElement>('dialog[open]')!.close();
		r.unmount();
	});

	test('Change password: the same, through Settings › Account', async () => {
		const r = render(AccountSettings);
		await waitFor(() => expect(r.container.querySelector('[data-account-row="password"]')).not.toBeNull());
		await fireEvent.click(r.container.querySelector('[data-account-row="password"]')!);
		const form = (await waitFor(() => document.querySelector<HTMLElement>('dialog[open] [data-change-password]')!)) as HTMLElement;
		const fill = async (current: string) => {
			await fireEvent.input(form.querySelector('[data-current]')!, { target: { value: current } });
			await fireEvent.input(form.querySelector('[data-new]')!, { target: { value: 'a much longer new one' } });
			await fireEvent.input(form.querySelector('[data-confirm]')!, { target: { value: 'a much longer new one' } });
			await fireEvent.submit(form);
		};
		await fill('wrong password!');
		await waitFor(() => expect(document.querySelector('[data-change-error]')?.textContent).toBe('The current password or two-step code isn’t right.'));
		expect(postsTo('/account/password')).toHaveLength(1);
		gw.expired.add(mod.session.token);
		await fill(PASSWORD);
		await waitFor(() => expect(document.querySelector('[data-change-error]')?.textContent ?? '').toMatch(/renewed first/));
		expect(postsTo('/account/password')).toHaveLength(2);
		expect(mod.gatewayAuth.refreshes).toBe(1);
		document.querySelector<HTMLDialogElement>('dialog[open]')!.close();
		r.unmount();
	});

	test('ChangePassword on its own maps a 403 as before', async () => {
		const change = vi.fn(async () => Promise.reject(new ApiError(403, 'the current password or two-step code is wrong')));
		const r = render(ChangePassword, { username: 'demo', totp: false, change });
		await fireEvent.input(r.container.querySelector('[data-current]')!, { target: { value: 'x' } });
		await fireEvent.input(r.container.querySelector('[data-new]')!, { target: { value: 'a much longer new one' } });
		await fireEvent.input(r.container.querySelector('[data-confirm]')!, { target: { value: 'a much longer new one' } });
		await fireEvent.submit(r.container.querySelector('[data-change-password]')!);
		await settle();
		expect(r.container.querySelector('[data-change-error]')!.textContent).toBe('The current password or two-step code isn’t right.');
		expect(change).toHaveBeenCalledTimes(1);
		r.unmount();
	});
});
