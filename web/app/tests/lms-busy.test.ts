// LMS busy handling (docs/PROTOCOL.md): the gateway's busy 503 and events messages, the
// calm status, polling that never stacks and backs off while busy, and recovery.
import { readFileSync } from 'node:fs';
import { ApiError, BUSY_MESSAGE, callJson, isBusy } from '$lib/api/http';
import { EventsClient, parseEventMessage, type LmsHealth } from '$lib/api/events';
import { Api } from '$lib/api/rpc';
import { SessionManager, type AuthApi } from '$lib/api/session';
import type { Session } from '$lib/api/auth';
import { BUSY_POLL_MAX_MS, BusyGate, POLL_MS, Single } from '$lib/state/busy';
import { api, app, session } from '$lib/state/app.svelte';
import type { PlayerInfo } from '$lib/lms/types';

function json(status: number, body: unknown, headers: Record<string, string> = {}) {
	return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', ...headers } });
}
const busy503 = () => json(503, { error: 'music server is busy', busy: true }, { 'Retry-After': '2' });

function deferred<T>() {
	let resolve!: (v: T) => void;
	const promise = new Promise<T>((res) => (resolve = res));
	return { promise, resolve };
}
const fakeAuth = (): AuthApi => ({
	login: vi.fn(),
	refresh: vi.fn(async (): Promise<Session> => ({ access_token: 't1', expires_in: 900, user: { username: 'u', role: 'listener' }, device: { id: 1, name: 'x' } })),
	logout: vi.fn(),
});
const flush = () => new Promise((r) => setTimeout(r, 0));

afterEach(() => {
	vi.unstubAllGlobals();
	vi.restoreAllMocks();
	vi.useRealTimers();
});

describe('the busy 503', () => {
	test('is recognised, with a friendly message and its Retry-After', async () => {
		vi.stubGlobal('fetch', async () => busy503());
		const e = await callJson('/v1/rpc').catch((x) => x);
		expect(e).toBeInstanceOf(ApiError);
		expect(isBusy(e)).toBe(true);
		expect(e).toMatchObject({ status: 503, busy: true, retryAfter: 2, message: BUSY_MESSAGE });
	});

	test('other failures are not busy', async () => {
		const cases: Response[] = [
			json(503, { error: 'music server unavailable' }),
			json(503, { error: 'x', busy: 'true' }),
			json(503, { error: 'x', busy: 1 }),
			json(502, { error: 'x', busy: true }),
			json(500, { busy: true }),
			new Response('<html>503</html>', { status: 503 }),
		];
		for (const r of cases) {
			vi.stubGlobal('fetch', async () => r);
			const e = (await callJson('/v1/rpc').catch((x) => x)) as ApiError;
			expect(isBusy(e)).toBe(false);
			expect(e.message).not.toBe(BUSY_MESSAGE);
		}
		expect(isBusy(new Error('busy'))).toBe(false);
		expect(isBusy({ status: 503, busy: true })).toBe(false);
	});

});

describe('events: lms messages', () => {
	test('busy and ok are parsed', () => {
		expect(parseEventMessage('{"lms":"busy","since":1760000000000}')).toEqual({ lms: 'busy', since: 1760000000000 });
		expect(parseEventMessage('{"lms":"ok"}')).toEqual({ lms: 'ok' });
		// A missing or odd `since` still means busy.
		for (const s of ['', ',"since":"1"', ',"since":-1', ',"since":1.5', ',"since":1e300', ',"since":null']) {
			expect(parseEventMessage(`{"lms":"busy"${s}}`)).toEqual({ lms: 'busy', since: null });
		}
	});

	test('unknown or malformed lms messages are ignored', () => {
		for (const bad of ['{"lms":"down"}', '{"lms":true}', '{"lms":null}', '{"lms":["busy"]}', '{"lms":"BUSY"}', '{"lms":"busy","event":["play"],"player":null}', '{"lms":', '["lms","busy"]']) {
			expect(parseEventMessage(bad)).toBeNull();
		}
	});

	class FakeWs {
		static all: FakeWs[] = [];
		sent: string[] = [];
		onopen?: () => void;
		onmessage?: (m: { data: unknown }) => void;
		onclose?: (e: { code: number; reason: string }) => void;
		constructor(public url: string) {
			FakeWs.all.push(this);
		}
		send(s: string) {
			this.sent.push(s);
		}
		close() {}
	}

	test('the client reports them, assumes ok on subscribe, and never as events', async () => {
		FakeWs.all = [];
		const s = new SessionManager(fakeAuth());
		await s.restore();
		const health: LmsHealth[] = [];
		const events: unknown[] = [];
		const c = new EventsClient(s, { onEvent: (e) => events.push(e), onResync: () => {}, onLms: (h) => health.push(h) }, (u) => new FakeWs(u) as unknown as WebSocket);
		c.start();
		const ws = FakeWs.all[0]!;
		ws.onopen!();
		ws.onmessage!({ data: '{"subscribed":true}' });
		ws.onmessage!({ data: '{"lms":"busy","since":5}' });
		ws.onmessage!({ data: '{"lms":"weird"}' });
		ws.onmessage!({ data: '{"lms":"ok"}' });
		expect(health).toEqual([{ lms: 'ok' }, { lms: 'busy', since: 5 }, { lms: 'ok' }]);
		expect(events).toEqual([]);
		c.stop();
	});
});

describe('BusyGate', () => {
	test('polls freely until busy, then backs off, respecting Retry-After', () => {
		const g = new BusyGate();
		expect(g.mayPoll(0)).toBe(true);
		expect(g.markBusy(1000, 2)).toBe(true);
		expect(g.busy).toBe(true);
		expect(g.mayPoll(2999)).toBe(false);
		expect(g.mayPoll(3000)).toBe(true);
		// Refusals in a row double the gap, up to the cap.
		expect(g.markBusy(3000, 2)).toBe(false);
		expect(g.mayPoll(3000 + 2 * POLL_MS - 1)).toBe(false);
		expect(g.mayPoll(3000 + 2 * POLL_MS)).toBe(true);
		for (let i = 0; i < 10; i++) g.markBusy(10_000, 2);
		expect(g.mayPoll(10_000 + BUSY_POLL_MAX_MS - 1)).toBe(false);
		expect(g.mayPoll(10_000 + BUSY_POLL_MAX_MS)).toBe(true);
	});

	test('a long Retry-After is respected, but capped', () => {
		const g = new BusyGate();
		g.markBusy(0, 10);
		expect(g.mayPoll(9999)).toBe(false);
		expect(g.mayPoll(10_000)).toBe(true);
		const h = new BusyGate();
		h.markBusy(0, 300);
		expect(h.mayPoll(BUSY_POLL_MAX_MS)).toBe(true);
	});

	test('ok clears everything, and says whether it was busy', () => {
		const g = new BusyGate();
		expect(g.markOk()).toBe(false);
		g.markBusy(0, null, 123);
		expect(g.since).toBe(123);
		expect(g.markOk()).toBe(true);
		expect(g).toMatchObject({ busy: false, since: null });
		expect(g.mayPoll(0)).toBe(true);
		// The backoff starts again from the beginning.
		g.markBusy(0, 2);
		expect(g.mayPoll(POLL_MS)).toBe(true);
	});
});

describe('Single', () => {
	test('never overlaps; a call meanwhile runs once more afterwards', async () => {
		const gates: ReturnType<typeof deferred<void>>[] = [];
		let calls = 0;
		const s = new Single(() => {
			calls++;
			const d = deferred<void>();
			gates.push(d);
			return d.promise;
		});
		const a = s.run();
		expect(s.poll()).toBeNull(); // a poll while one runs: nothing
		void s.run();
		void s.run();
		expect(calls).toBe(1);
		gates[0]!.resolve();
		await flush();
		expect(calls).toBe(2); // the follow-up, once
		gates[1]!.resolve();
		await a;
		expect(calls).toBe(2);
		expect(s.running).toBe(false);
	});

	test('no follow-up when told not to (LMS busy); a failing task does not wedge it', async () => {
		let calls = 0;
		let allow = false;
		const s = new Single(
			async () => {
				calls++;
				await flush();
				throw new Error('x');
			},
			() => allow,
		);
		const a = s.run();
		void s.run();
		await a;
		expect(calls).toBe(1);
		allow = true;
		await s.run();
		expect(calls).toBe(2);
		expect(s.running).toBe(false);
	});
});

describe('Api health reports', () => {
	test('busy refusals and successes are reported; other failures are not', async () => {
		const s = new SessionManager(fakeAuth());
		await s.restore();
		const a = new Api(s);
		const seen: (ApiError | null)[] = [];
		a.onHealth = (e) => seen.push(e);
		vi.stubGlobal('fetch', async () => busy503());
		await expect(a.rpc('', ['players'])).rejects.toMatchObject({ busy: true });
		vi.stubGlobal('fetch', async () => json(502, { error: 'music server unavailable' }));
		await expect(a.rpc('', ['players'])).rejects.toMatchObject({ status: 502 });
		vi.stubGlobal('fetch', async () => json(200, { result: {} }));
		await a.rpc('', ['players']);
		expect(seen.map((e) => (e ? 'busy' : 'ok'))).toEqual(['busy', 'ok']);
	});
});

describe('the app while LMS is busy', () => {
	const PLAYER = '02:00:00:00:00:01';
	let calls: { cmd: unknown[] }[] = [];
	let respond: (cmd: unknown[]) => Response | Promise<Response>;

	beforeEach(() => {
		calls = [];
		vi.spyOn(session, 'authed').mockImplementation((fn) => fn('tok'));
		vi.stubGlobal('fetch', async (_u: string, init: RequestInit) => {
			const b = JSON.parse(init.body as string);
			calls.push(b);
			return respond(b.cmd);
		});
		app.players = [{ playerid: PLAYER, name: 'Test player' } as PlayerInfo];
		app.playerId = PLAYER;
		app.error = '';
		app.notice = '';
		app.status = 'signed-in';
	});

	afterEach(async () => {
		// Leave it not busy for the next test.
		respond = () => json(200, { result: { players_loop: [{ playerid: PLAYER, name: 'Test player' }] } });
		await app.refresh();
		await new Promise((r) => setTimeout(r, 200));
		app.status = 'unknown';
	});

	test('a busy 503 is a calm status, not an error', async () => {
		respond = () => busy503();
		await app.refresh();
		expect(app.serverBusy).toBe(true);
		expect(app.error).toBe('');
	});

	test('becoming busy clears a stale error; real errors still show', async () => {
		app.error = 'music server unavailable';
		respond = () => busy503();
		await app.refresh();
		expect(app.error).toBe('');
		respond = () => json(403, { error: 'command not allowed' });
		await app.refresh();
		expect(app.error).toBe('command not allowed');
	});

	test('a command refused as busy is not replayed; the connection status says why (no per-command note)', async () => {
		respond = () => busy503();
		await app.send(['pause', '1']);
		expect(app.notice).toBe('');
		expect(app.error).toBe('');
		expect(app.reach).toBe('busy');
		await new Promise((r) => setTimeout(r, 200));
		expect(calls.filter((c) => c.cmd[0] === 'pause')).toHaveLength(1);
		// While busy, further commands aren't even sent (the controls are disabled).
		await app.send(['pause', '0']);
		expect(calls.filter((c) => c.cmd[0] === 'pause')).toHaveLength(1);
	});

	test('the first success clears the status and catches up at once', async () => {
		respond = () => busy503();
		await app.refresh();
		await app.send(['pause', '1']);
		expect(app.serverBusy).toBe(true);
		calls = [];
		respond = (cmd) => json(200, { result: cmd[0] === 'players' ? { players_loop: [{ playerid: PLAYER, name: 'Test player' }] } : { mode: 'play' } });
		await app.refresh();
		expect(app.serverBusy).toBe(false);
		expect(app.notice).toBe(''); // the busy note goes with it
		await new Promise((r) => setTimeout(r, 200));
		expect(calls.some((c) => c.cmd[0] === 'players')).toBe(true);
	});

	test('status requests never stack', async () => {
		const gate = deferred<Response>();
		respond = () => gate.promise;
		const first = app.refresh();
		void app.refresh();
		void app.refresh();
		await flush();
		expect(calls).toHaveLength(1);
		respond = () => json(200, { result: { mode: 'stop' } });
		gate.resolve(json(200, { result: { mode: 'play' } }));
		await first;
		expect(calls).toHaveLength(2); // one follow-up for the calls made meanwhile
	});

	test('no follow-up request while busy', async () => {
		const gate = deferred<Response>();
		respond = () => gate.promise;
		const first = app.refresh();
		void app.refresh();
		gate.resolve(busy503());
		await first;
		expect(calls).toHaveLength(1);
	});
});

describe('wiring', () => {
	const src = readFileSync(`${__dirname}/../src/lib/state/app.svelte.ts`, 'utf8');
	const layout = readFileSync(`${__dirname}/../src/routes/+layout.svelte`, 'utf8');

	test('the poll goes through the gate and never stacks', () => {
		expect(src).toMatch(/this\.#busy\.mayPoll\(Date\.now\(\)\)\) void this\.#refresher\.poll\(\)/);
		expect(src).toMatch(/onLms: \(h\) => this\.#onLms\(h\)/);
		expect(src).toMatch(/api\.onHealth = /);
	});

	test('the layout shows one calm connection status (busy included), at every width', () => {
		expect(layout).toMatch(/\{#if controls\.message\}\s*<p class="chip" class:busy=\{app\.reach === 'busy'\} role="status"/);
		expect(layout).not.toMatch(/app\.serverBusy\}\s*<div class="banner/);
	});
});
