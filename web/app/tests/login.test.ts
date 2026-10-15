// Sign-in (review M20): each refusal from the gateway reads as what it is; only a request that
// never reached the server says to check the connection.
import { fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { ApiError, BUSY_MESSAGE } from '$lib/api/http';

const signIn = vi.fn<(u: string, p: string, c?: string) => Promise<void>>();
vi.mock('$lib/state/app.svelte', () => ({ session: { signIn: (...a: [string, string, string?]) => signIn(...a) } }));
const { default: Login } = await import('$lib/components/Login.svelte');

const CONNECTION = 'Could not reach the server. Check your connection.';

async function attempt(err: unknown): Promise<string> {
	signIn.mockReset();
	signIn.mockRejectedValue(err);
	const r = render(Login);
	await fireEvent.input(screen.getByLabelText('Username'), { target: { value: 'demo' } });
	await fireEvent.input(screen.getByLabelText('Password'), { target: { value: 'pw' } });
	await fireEvent.submit(r.container.querySelector('form')!);
	for (let i = 0; i < 4; i++) await tick();
	const text = (await screen.findByRole('alert')).textContent ?? '';
	r.unmount();
	return text;
}

test('403 (the Origin or CSRF check failed) says the server refused, and hints at the proxy', async () => {
	const text = await attempt(new ApiError(403, 'forbidden'));
	expect(text).toBe('The server refused this sign-in. If the app is behind a proxy, check that it passes the Origin header.');
});

test('503 says the music server is busy', async () => {
	expect(await attempt(new ApiError(503, 'busy; try again shortly'))).toBe('The music server is busy. Try again shortly.');
	expect(await attempt(new ApiError(503, BUSY_MESSAGE, false, 1, { busy: true }))).toBe('The music server is busy. Try again shortly.');
});

test('a server error (5xx) says the server had a problem', async () => {
	for (const s of [500, 502, 504]) expect(await attempt(new ApiError(s, 'internal error'))).toBe('The server had a problem. Try again shortly.');
});

test('other refusals name their status rather than blaming the connection', async () => {
	const text = await attempt(new ApiError(404, 'HTTP 404'));
	expect(text).not.toBe(CONNECTION);
	expect(text).toContain('HTTP 404');
});

test('only a network failure says to check the connection', async () => {
	expect(await attempt(new TypeError('Failed to fetch'))).toBe(CONNECTION);
	expect(await attempt(new Error('odd'))).not.toBe(CONNECTION);
});

test('the existing answers are unchanged: 401, 429 and a two-step code', async () => {
	expect(await attempt(new ApiError(401, 'invalid credentials'))).toBe('Username or password is not right.');
	expect(await attempt(new ApiError(429, 'too many'))).toBe('Too many attempts. Wait a while and try again.');
	expect(await attempt(new ApiError(401, 'two-step code required', true))).toBe('Enter the code from your authenticator app.');
});
