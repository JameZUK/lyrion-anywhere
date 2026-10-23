export {};
// The sleep timer in the app state: what it sends LMS, re-arming an "end of …" timer when the
// end moves, "sleep all", cancelling, and this device's fade-out.
vi.mock('#lib/api/session', () => {
	class SessionManager {
		current = null;
		token = 't';
		subscribe() {
			return () => {};
		}
		authed<T>(fn: (t: string) => Promise<T>) {
			return fn('t');
		}
		renew() {
			return Promise.resolve(true);
		}
	}
	return { SessionManager };
});

const ME = '02:00:00:00:00:01';
const KITCHEN = '02:00:00:00:00:02';
const calls: { player: string; cmd: unknown[] }[] = [];
let status: Record<string, unknown> = {};
let upcoming: Record<string, unknown>[] = [];
let refuseSleep = false;
vi.mock('#lib/api/rpc', () => {
	class Api {
		async rpc(player: string, cmd: unknown[]) {
			calls.push({ player, cmd });
			if (cmd[0] === 'players') return { players_loop: [{ playerid: ME, name: 'Me' }, { playerid: KITCHEN, name: 'Kitchen', connected: 1 }] };
			if (cmd[0] === 'sleep' && refuseSleep) throw new Error('refused');
			if (cmd[0] === 'status' && cmd[3] === 'tags:de') return { playlist_loop: upcoming };
			if (cmd[0] === 'status') return status;
			return {};
		}
		art() {
			return Promise.resolve('blob:x');
		}
		clearArt() {}
	}
	return { Api };
});

const { app } = await import('#lib/state/app.svelte');
const sleeps = () => calls.filter((c) => c.cmd[0] === 'sleep').map((c) => [c.player, c.cmd[1]]);
const fades: [number | null, number][] = [];

beforeEach(async () => {
	calls.length = 0;
	fades.length = 0;
	refuseSleep = false;
	vi.useRealTimers();
	app.onDeviceSleep = (at, f) => fades.push([at, f]);
	app.armed = null;
	app.playerId = ME;
	status = {
		mode: 'play',
		time: 100,
		duration: 300,
		playlist_cur_index: 0,
		playlist_tracks: 3,
		playlist_loop: [{ id: 1, album_id: 7, duration: 300 }],
	};
	upcoming = [
		{ id: 2, album_id: 7, duration: 200 },
		{ id: 3, album_id: 8, duration: 400 },
	];
	await app.loadPlayers();
	await app.refresh();
	calls.length = 0;
});

test('a fixed length is LMS sleep in seconds, on this player', async () => {
	expect(await app.setSleep({ kind: 'minutes', minutes: 30 })).toBe('');
	expect(sleeps()).toEqual([[ME, 1800]]);
	expect(app.armed).toBeNull(); // nothing to keep in step
});

test('end of album and queue are worked out from the queue', async () => {
	await app.setSleep({ kind: 'album' });
	expect(sleeps()).toEqual([[ME, 200 + 200]]);
	expect(calls.some((c) => c.cmd[0] === 'status' && c.cmd[1] === 1 && c.cmd[3] === 'tags:de')).toBe(true);
	expect(app.armed?.end).toBe('album');
	calls.length = 0;
	await app.setSleep({ kind: 'queue' });
	expect(sleeps()).toEqual([[ME, 200 + 200 + 400]]);
});

test('a stream with no length: refused with a reason, nothing sent', async () => {
	status = { ...status, duration: 0, playlist_loop: [{ title: 'Radio', remote: 1 }] };
	await app.refresh();
	calls.length = 0;
	expect(await app.setSleep({ kind: 'track' })).toMatch(/Can't tell/);
	expect(sleeps()).toEqual([]);
});

test('an armed timer is set again when the end moves (a seek)', async () => {
	await app.setSleep({ kind: 'track' });
	expect(sleeps()).toEqual([[ME, 200]]);
	calls.length = 0;
	status = { ...status, will_sleep_in: 199 };
	await app.refresh();
	await vi.waitFor(() => expect(calls.some((c) => c.cmd[0] === 'status')).toBe(true));
	expect(sleeps()).toEqual([]); // in step
	// The user seeks to near the end: the timer follows.
	status = { ...status, time: 280, will_sleep_in: 198 };
	await app.refresh();
	await vi.waitFor(() => expect(sleeps()).toEqual([[ME, 20]]));
	expect(app.armed?.end).toBe('track');
});

test('a queue edit moves the end of the album: set again', async () => {
	await app.setSleep({ kind: 'album' });
	calls.length = 0;
	upcoming = [{ id: 3, album_id: 8, duration: 400 }]; // the rest of the album was removed
	app.queueVersion++;
	status = { ...status, playlist_tracks: 2, will_sleep_in: 399 };
	await app.refresh();
	await vi.waitFor(() => expect(sleeps()).toEqual([[ME, 200]]));
});

test('cancelled elsewhere (LMS no longer reports it): disarmed, never set again', async () => {
	await app.setSleep({ kind: 'track' });
	app.armed = { ...app.armed!, setAt: Date.now() - 60_000 };
	calls.length = 0;
	status = { ...status, time: 250 }; // no will_sleep_in
	await app.refresh();
	await vi.waitFor(() => expect(app.armed).toBeNull());
	expect(sleeps()).toEqual([]);
});

test('sleep all: every connected player; cancel turns each off', async () => {
	await app.setSleep({ kind: 'minutes', minutes: 15 }, 'all');
	expect(sleeps().sort()).toEqual([
		[ME, 900],
		[KITCHEN, 900],
	]);
	calls.length = 0;
	await app.setSleep({ kind: 'track' }, 'all');
	calls.length = 0;
	expect(await app.cancelSleep()).toBe('');
	expect(sleeps().sort()).toEqual([
		[ME, 0],
		[KITCHEN, 0],
	]);
	expect(app.armed).toBeNull();
});

test("this device's own timer fades its output (and cancelling lifts it)", async () => {
	const mac = '02:00:00:00:00:01';
	Object.defineProperty(app, 'deviceMac', { get: () => mac, configurable: true });
	try {
		await app.setSleep({ kind: 'minutes', minutes: 30 });
		expect(fades.at(-1)?.[1]).toBe(app.sleepFade);
		expect(fades.at(-1)?.[0]).toBeGreaterThan(Date.now() + 1790_000);
		// LMS reports the timer (set from anywhere): the fade follows it.
		status = { ...status, will_sleep_in: 600 };
		await app.refresh();
		expect(fades.at(-1)?.[0]).toBeLessThan(Date.now() + 601_000);
		await app.cancelSleep();
		expect(fades.at(-1)).toEqual([null, 0]);
	} finally {
		delete (app as unknown as Record<string, unknown>).deviceMac;
	}
});

test('LMS refusing the timer is reported, not thrown', async () => {
	refuseSleep = true;
	expect(await app.setSleep({ kind: 'minutes', minutes: 30 })).toMatch(/Couldn't set the sleep timer: refused/);
	expect(await app.cancelSleep()).toMatch(/Couldn't turn the sleep timer off/);
});

test('the fade setting is checked and kept', () => {
	app.setSleepFade(90);
	expect(app.sleepFade).toBe(90);
	app.setSleepFade(5000);
	expect(app.sleepFade).toBe(30);
	app.setSleepFade(0);
	expect(app.sleepFade).toBe(0);
	app.setSleepFade(30);
});
