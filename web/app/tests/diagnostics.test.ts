import { Reporter, buildReport, diagnosticsSender } from '$lib/player/diagnostics';
import { SessionManager } from '$lib/api/session';
import { ApiError } from '$lib/api/http';

afterEach(() => vi.useRealTimers());

const input = {
	capabilities: { managedMediaSource: true, mediaSource: true, audioWorklet: true, audioSession: true, types: [], userAgent: 'UA' },
	environment: { appleMobile: true },
	preference: 'auto' as const,
	planned: { mode: 'media' as const, source: 'managed' as const, mime: 'audio/mp4; codecs="fLaC"', reason: 'iPad' },
	active: null,
	device: { state: 'connected', mac: '02:4c:41:00:00:09', name: 'Safari on iOS', detail: { output: 'Playing' } },
	log: Array.from({ length: 200 }, (_, i) => `line ${i}`),
	page: { visibility: 'visible', standalone: false },
};

test('reports carry the playback facts and a bounded log', () => {
	const r = buildReport(input, new Date('2026-10-10T16:00:00Z'));
	expect(r.playback.planned.mime).toBe('audio/mp4; codecs="fLaC"');
	expect(r.userAgent).toBe('UA');
	expect(r.log).toHaveLength(60);
	expect(r.at).toBe('2026-10-10T16:00:00.000Z');
	expect(JSON.stringify(r).length).toBeLessThan(32 * 1024);
});

test('reports are batched, with urgent ones sent at once', async () => {
	vi.useFakeTimers();
	const sent: boolean[] = [];
	let t = 0;
	const r = new Reporter(async (_b, urgent) => void sent.push(urgent), () => ({}), 15_000, () => t);
	r.schedule();
	r.schedule();
	r.schedule();
	await vi.advanceTimersByTimeAsync(0);
	expect(sent).toEqual([false]);
	t = 1000;
	r.schedule();
	await vi.advanceTimersByTimeAsync(5000);
	expect(sent).toHaveLength(1); // waits out the gap
	r.urgent();
	await vi.advanceTimersByTimeAsync(0);
	expect(sent).toEqual([false, true]);
	vi.useRealTimers();
});

describe('diagnosticsSender', () => {
	const ok = () => new Response(null, { status: 204 });
	test('posts the report with the bearer token and CSRF header', async () => {
		const f = vi.fn(async () => ok());
		const send = diagnosticsSender((fn) => fn('t1'), f as unknown as typeof fetch);
		await send('{"a":1}', true);
		const [url, init] = f.mock.calls[0] as unknown as [string, RequestInit];
		expect(url).toBe('/v1/diagnostics');
		expect(init).toMatchObject({ method: 'POST', body: '{"a":1}', keepalive: true });
		expect(init.headers).toMatchObject({ Authorization: 'Bearer t1', 'X-Anywhere-Client': '1' });
	});

	// Regression: the response was never checked, so a refused (expired) token was neither
	// renewed nor noticed.
	test('a 401 goes through the session renewal and is retried with the new token', async () => {
		const auth = {
			login: vi.fn(),
			logout: vi.fn(async () => {}),
			refresh: vi.fn(async () => ({ access_token: auth.refresh.mock.calls.length === 1 ? 't1' : 't2', expires_in: 900, user: { username: 'u', role: 'listener' as const }, device: { id: 1, name: 'd' } })),
		};
		const s = new SessionManager(auth, undefined);
		await s.restore();
		const seen: string[] = [];
		const f = vi.fn(async (_u: unknown, init: RequestInit) => {
			const t = (init.headers as Record<string, string>).Authorization!;
			seen.push(t);
			return t === 'Bearer t1' ? new Response('{}', { status: 401 }) : ok();
		});
		await diagnosticsSender((fn) => s.authed(fn), f as unknown as typeof fetch)('{}', false);
		expect(seen).toEqual(['Bearer t1', 'Bearer t2']);
	});

	test('other failures are errors, not silent successes', async () => {
		const send = diagnosticsSender((fn) => fn('t'), (async () => new Response('', { status: 500 })) as unknown as typeof fetch);
		await expect(send('{}', false)).rejects.toMatchObject({ status: 500 });
		const s403 = diagnosticsSender((fn) => fn('t'), (async () => new Response('', { status: 403 })) as unknown as typeof fetch);
		await expect(s403('{}', false)).rejects.toBeInstanceOf(ApiError);
	});
});

// Review (Web nit): every tab hide posted a report, even with nothing new to say.
test('hiding the tab again with nothing new sends nothing; something new is sent', async () => {
	const sent: string[] = [];
	let state = 'connected';
	let at = 0;
	let visibility = 'hidden';
	const r = new Reporter(async (b) => void sent.push(b), () => ({ at: at++, page: { visibility }, device: { state } }));
	r.urgent();
	await Promise.resolve();
	expect(sent).toHaveLength(1);
	visibility = 'visible';
	r.urgent(); // only the time and the page's visibility differ
	visibility = 'hidden';
	r.urgent();
	await Promise.resolve();
	expect(sent).toHaveLength(1);
	state = 'reconnecting';
	r.urgent();
	await Promise.resolve();
	expect(sent).toHaveLength(2);
	expect(JSON.parse(sent[1]!).device.state).toBe('reconnecting');
});

test('a failed send is tried again on the next hide, even with nothing new', async () => {
	let fail = true;
	const sent: string[] = [];
	const r = new Reporter(
		async (b) => {
			if (fail) throw new Error('offline');
			sent.push(b);
		},
		() => ({ device: { state: 'connected' } }),
	);
	r.urgent();
	await new Promise((res) => setTimeout(res, 0));
	fail = false;
	r.urgent();
	await new Promise((res) => setTimeout(res, 0));
	expect(sent).toHaveLength(1);
});

test('a failing send never throws', async () => {
	const r = new Reporter(async () => Promise.reject(new Error('offline')), () => ({}));
	r.urgent();
	await new Promise((res) => setTimeout(res, 0));
});
