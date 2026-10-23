import { LOG_LINES, LogLimiter, Reporter, buildReport, describeCommand, describePageCommand, diagnosticsSender, recentLog } from '#lib/player/diagnostics';
import { SessionManager } from '#lib/api/session';
import { ApiError } from '#lib/api/http';

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
	expect(r.log).toHaveLength(LOG_LINES);
	expect(r.log[0]).toBe('line 0'); // the newest first
	expect(r.at).toBe('2026-10-10T16:00:00.000Z');
	expect(JSON.stringify(r).length).toBeLessThan(32 * 1024);
});

// The gateway refuses a report over 32 KiB: a log of long lines is cut by size, not only count.
test('a log of long lines is cut to fit, newest first', () => {
	const long = Array.from({ length: 200 }, (_, i) => `${i} ${'é'.repeat(300)}`);
	const r = buildReport({ ...input, log: long });
	expect(r.log.length).toBeGreaterThan(10);
	expect(r.log.length).toBeLessThan(LOG_LINES);
	expect(r.log[0]).toBe(long[0]);
	expect(new TextEncoder().encode(JSON.stringify(r)).length).toBeLessThan(32 * 1024);
	expect(recentLog([])).toEqual([]);
});

describe('LogLimiter', () => {
	test('a kind is logged at most once per interval, then says how many were left out', () => {
		const l = new LogLimiter(5_000);
		expect(l.pass('media: waiting at 1.0 s', 'media:waiting', 0)).toBe('media: waiting at 1.0 s');
		expect(l.pass('media: waiting at 2.0 s', 'media:waiting', 1_000)).toBeNull();
		expect(l.pass('media: waiting at 3.0 s', 'media:waiting', 4_999)).toBeNull();
		// Another kind is not held back by it.
		expect(l.pass('media: stalled at 3.0 s', 'media:stalled', 4_999)).toBe('media: stalled at 3.0 s');
		expect(l.pass('media: waiting at 9.0 s', 'media:waiting', 5_000)).toBe('media: waiting at 9.0 s (and 2 more like it)');
		expect(l.pass('media: waiting at 20.0 s', 'media:waiting', 20_000)).toBe('media: waiting at 20.0 s');
	});

	test('lines without a kind always go through', () => {
		const l = new LogLimiter(5_000);
		for (let i = 0; i < 5; i++) expect(l.pass('player: Playing → Paused', undefined, i)).toBe('player: Playing → Paused');
		expect(l.pass('x', '', 0)).toBe('x');
	});

	test('a clock that goes backwards never silences a kind for good', () => {
		const l = new LogLimiter(5_000);
		expect(l.pass('a', 'k', 10_000)).toBe('a');
		expect(l.pass('b', 'k', 1_000)).toBe('b');
	});

	test('many kinds are remembered within a bound, and still limited', () => {
		const l = new LogLimiter(5_000);
		for (let i = 0; i < 1000; i++) l.pass(`line ${i}`, `kind ${i}`, 0);
		expect(l.pass('again', 'kind 999', 1)).toBeNull();
		expect(l.pass('again', 'kind 0', 1)).toBe('again'); // forgotten: logged again, harmlessly
	});
});

describe('page commands', () => {
	test('each command the page sends by itself is described in a few words', () => {
		expect(describePageCommand(['pause', 1], 'the system paused the media element')).toBe('page: asked LMS to pause (the system paused the media element)');
		expect(describeCommand(['pause', 0])).toBe('resume');
		expect(describeCommand(['pause'])).toBe('pause or resume');
		expect(describeCommand(['play'])).toBe('play');
		expect(describeCommand(['stop'])).toBe('stop');
		expect(describeCommand(['time', 61.25])).toBe('seek to 61.3 s');
		expect(describeCommand(['time', '42'])).toBe('seek to 42 s');
		expect(describeCommand(['playlist', 'index', 3])).toBe('play queue entry 3');
		expect(describeCommand(['mixer', 'volume', 100])).toBe('set the volume to 100%');
		expect(describeCommand(['status'])).toBe("run 'status'");
	});

	test('free text in a command never reaches the log', () => {
		for (const c of [['playlist', 'play', '/srv/music/a.flac'], ['time', 'abc'], ['mixer', 'volume', '+5; rm'], ['Search Me'], [42], []]) {
			const d = describeCommand(c as (string | number)[]);
			expect(d).not.toMatch(/srv|music|rm|Search|abc|;/);
		}
		expect(describeCommand(['playlist', 'play', '/srv/music/a.flac'])).toBe("run 'playlist'");
		expect(describeCommand(['time', 'abc'])).toBe("run 'time'");
		expect(describeCommand([42])).toBe('run a command');
	});
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
