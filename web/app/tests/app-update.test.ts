// A new version of the app applies itself at once, unless this device is playing: then it is
// offered in the status slot and applies itself when playback here stops. "Later" holds it.
import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { AppUpdate, CHECK_EVERY_MS, decideUpdate, playingHere, type SwContainer, type SwRegistration, type SwWorker } from '#lib/pwa/update.svelte';
import { MSG } from '#lib/pwa/sw-handlers';
import { statusOf, UPDATE_TEXT } from '#lib/state/status';

vi.mock('#lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));

class Target {
	listeners = new Map<string, Array<{ fn: (e: never) => void; once?: boolean }>>();
	addEventListener(type: string, fn: (e: never) => void, opts?: { once?: boolean }) {
		this.listeners.set(type, [...(this.listeners.get(type) ?? []), { fn, once: opts?.once }]);
	}
	emit(type: string, e: unknown = {}) {
		const ls = this.listeners.get(type) ?? [];
		this.listeners.set(
			type,
			ls.filter((l) => !l.once),
		);
		for (const l of ls) l.fn(e as never);
	}
}

class Worker extends Target implements SwWorker {
	state = 'installing';
	posted: unknown[] = [];
	postMessage(m: unknown) {
		this.posted.push(m);
	}
}

class Registration extends Target implements SwRegistration {
	waiting: Worker | null = null;
	installing: Worker | null = null;
	updates = 0;
	async update() {
		this.updates++;
	}
}

function container(reg: Registration, controlled = true) {
	const posted: unknown[] = [];
	const c = new Target() as Target & SwContainer & { posted: unknown[] };
	Object.assign(c, {
		posted,
		controller: controlled ? { postMessage: (m: unknown) => posted.push(m) } : null,
		getRegistration: async () => reg,
	});
	return c;
}

const flush = async () => {
	for (let i = 0; i < 3; i++) await Promise.resolve();
};

describe('decideUpdate', () => {
	const base = { waiting: true, controlled: true, playing: false, dismissed: false };
	test.each([
		[{ ...base, waiting: false }, 'none'],
		[{ ...base, waiting: false, playing: true }, 'none'],
		[{ ...base, controlled: false }, 'none'], // a first install: nothing to upgrade from
		[{ ...base, controlled: false, playing: true }, 'none'],
		[base, 'apply'],
		[{ ...base, playing: true }, 'offer'],
		[{ ...base, dismissed: true }, 'hold'],
		[{ ...base, dismissed: true, playing: true }, 'hold'],
	] as const)('%o → %s', (s, want) => {
		expect(decideUpdate(s)).toBe(want);
	});
});

describe('playingHere', () => {
	const idle = { downloads: false, deviceState: 'connected', thisDevice: true, mode: 'stop', output: 'Stopped' };
	test('this device playing, or paused mid-track, as the controlled player', () => {
		expect(playingHere({ ...idle, mode: 'play' })).toBe(true);
		expect(playingHere({ ...idle, mode: 'pause' })).toBe(true);
		expect(playingHere({ ...idle, deviceState: 'reconnecting', mode: 'play' })).toBe(true);
		expect(playingHere({ ...idle, deviceState: 'starting', mode: 'pause' })).toBe(true);
	});
	test('sound coming out here, whichever player is being controlled', () => {
		expect(playingHere({ ...idle, thisDevice: false, mode: undefined, output: 'Playing' })).toBe(true);
	});
	test('the downloads player in use', () => {
		expect(playingHere({ ...idle, downloads: true, deviceState: 'off' })).toBe(true);
	});
	test('nothing to interrupt', () => {
		expect(playingHere(idle)).toBe(false);
		expect(playingHere({ ...idle, mode: undefined, output: undefined })).toBe(false);
		// Another player is controlled and this one is silent.
		expect(playingHere({ ...idle, thisDevice: false, mode: 'play' })).toBe(false);
		// The player here is switched off, stopped or failed: LMS's mode for it does not matter.
		for (const deviceState of ['off', 'stopped', 'error', 'nonsense']) {
			expect(playingHere({ ...idle, deviceState, mode: 'play', output: 'Playing' })).toBe(false);
		}
	});
});

/** An update found while the page is open, then installed. */
function install(reg: Registration) {
	const w = new Worker();
	reg.installing = w;
	reg.emit('updatefound');
	w.state = 'installed';
	reg.waiting = w;
	reg.installing = null;
	w.emit('statechange');
	return w;
}

/** A fake hourly timer. */
function timer() {
	const t = { fn: null as (() => void) | null, ms: 0, stopped: false };
	const every = (fn: () => void, ms: number) => {
		t.fn = fn;
		t.ms = ms;
		return () => void (t.stopped = true);
	};
	return { t, every };
}

describe('AppUpdate', () => {
	test('a version waiting at launch applies at once: it takes over, then the page reloads', async () => {
		const reg = new Registration();
		const waiting = new Worker();
		waiting.state = 'installed';
		reg.waiting = waiting;
		const c = container(reg);
		const reload = vi.fn();
		const u = new AppUpdate();
		u.start('v1', { container: c, reload, every: timer().every });
		await flush();
		// The page said hello, so the worker can tidy what no tab needs.
		expect(c.posted).toEqual([{ type: MSG.hello }]);
		expect(waiting.posted).toEqual([{ type: MSG.skipWaiting }]);
		expect(u.offered).toBe(false);
		expect(reload).not.toHaveBeenCalled(); // not before the new version controls this page
		c.emit('controllerchange');
		expect(reload).toHaveBeenCalledOnce();
		// The worker's own "activated" afterwards does not reload a second time.
		waiting.state = 'activated';
		waiting.emit('statechange');
		expect(reload).toHaveBeenCalledOnce();
	});

	test('the new worker becoming active reloads even without a controllerchange', async () => {
		const reg = new Registration();
		const waiting = new Worker();
		reg.waiting = waiting;
		const reload = vi.fn();
		const u = new AppUpdate();
		u.start('v1', { container: container(reg), reload, every: timer().every });
		await flush();
		waiting.state = 'activating';
		waiting.emit('statechange');
		expect(reload).not.toHaveBeenCalled();
		waiting.state = 'activated';
		waiting.emit('statechange');
		expect(reload).toHaveBeenCalledOnce();
	});

	test('an update found while open and idle applies itself once installed', async () => {
		const reg = new Registration();
		const u = new AppUpdate();
		const c = container(reg);
		const reload = vi.fn();
		u.start('v1', { container: c, reload, every: timer().every });
		await flush();
		const w = install(reg);
		expect(w.posted).toEqual([{ type: MSG.skipWaiting }]);
		expect(u.offered).toBe(false);
		c.emit('controllerchange');
		expect(reload).toHaveBeenCalledOnce();
	});

	test('while this device plays it is offered, and applies itself when playback stops', async () => {
		const reg = new Registration();
		const u = new AppUpdate();
		const reload = vi.fn();
		const c = container(reg);
		u.start('v1', { container: c, reload, every: timer().every });
		await flush();
		u.setPlaying(true);
		const w = install(reg);
		expect(u.offered).toBe(true);
		expect(w.posted).toEqual([]); // the playback carries on
		u.setPlaying(true); // no change
		expect(w.posted).toEqual([]);
		u.setPlaying(false);
		expect(w.posted).toEqual([{ type: MSG.skipWaiting }]);
		c.emit('controllerchange');
		expect(reload).toHaveBeenCalledOnce();
	});

	test('"Reload" while playing applies it at once, and only once', async () => {
		const reg = new Registration();
		const u = new AppUpdate();
		u.start('v1', { container: container(reg), reload: vi.fn(), every: timer().every });
		await flush();
		u.setPlaying(true);
		const w = install(reg);
		u.apply();
		u.apply();
		u.setPlaying(false);
		expect(w.posted).toEqual([{ type: MSG.skipWaiting }]);
	});

	test('a first install (no controller yet) is not an update: nothing offered or applied', async () => {
		const reg = new Registration();
		const w = new Worker();
		reg.waiting = w;
		const reload = vi.fn();
		const u = new AppUpdate();
		u.start('v1', { container: container(reg, false), reload, every: timer().every });
		await flush();
		expect(u.offered).toBe(false);
		expect(w.posted).toEqual([]);
		expect(reload).not.toHaveBeenCalled();
	});

	test('"Later" holds the automatic apply until the next new version', async () => {
		const reg = new Registration();
		const u = new AppUpdate();
		u.start('v1', { container: container(reg), reload: vi.fn(), every: timer().every });
		await flush();
		u.setPlaying(true);
		const first = install(reg);
		u.dismiss();
		expect(u.offered).toBe(false);
		u.setPlaying(false); // stopped, but the person said "Later"
		expect(first.posted).toEqual([]);
		u.setPlaying(true);
		const next = install(reg);
		expect(u.offered).toBe(true); // a newer version: offered again
		u.setPlaying(false);
		expect(next.posted).toEqual([{ type: MSG.skipWaiting }]);
	});

	test('"Later" lasts until the next launch only', async () => {
		const reg = new Registration();
		const w = new Worker();
		reg.waiting = w;
		const relaunch = new AppUpdate(); // a new page: no memory of the dismissal
		relaunch.start('v1', { container: container(reg), reload: vi.fn(), every: timer().every });
		await flush();
		expect(w.posted).toEqual([{ type: MSG.skipWaiting }]);
	});

	test('the page tells the worker its version (and nothing for other messages)', async () => {
		const c = container(new Registration());
		const u = new AppUpdate();
		u.start('build-7', { container: c, reload: vi.fn(), every: timer().every });
		const replies: unknown[] = [];
		const port = { postMessage: (m: unknown) => replies.push(m) };
		c.emit('message', { data: { type: MSG.ask }, ports: [port] });
		c.emit('message', { data: { type: 'other' }, ports: [port] });
		c.emit('message', { data: null, ports: [port] });
		c.emit('message', { data: { type: MSG.ask }, ports: [] });
		expect(replies).toEqual([{ type: MSG.reply, version: 'build-7' }]);
	});

	test('every launch looks for a new version, then every hour while open', async () => {
		const reg = new Registration();
		const { t, every } = timer();
		const u = new AppUpdate();
		u.start('v1', { container: container(reg), reload: vi.fn(), every });
		await flush();
		expect(reg.updates).toBe(1);
		expect(t.ms).toBe(CHECK_EVERY_MS);
		t.fn!();
		t.fn!();
		expect(reg.updates).toBe(3);
		u.stop();
		expect(t.stopped).toBe(true);
	});

	test('the real hourly timer runs on the clock', async () => {
		vi.useFakeTimers();
		try {
			const reg = new Registration();
			const u = new AppUpdate();
			u.start('v1', { container: container(reg), reload: vi.fn() });
			await vi.advanceTimersByTimeAsync(0);
			expect(reg.updates).toBe(1);
			await vi.advanceTimersByTimeAsync(CHECK_EVERY_MS - 1);
			expect(reg.updates).toBe(1);
			await vi.advanceTimersByTimeAsync(1);
			expect(reg.updates).toBe(2);
			await vi.advanceTimersByTimeAsync(2 * CHECK_EVERY_MS);
			expect(reg.updates).toBe(4);
			u.stop();
			await vi.advanceTimersByTimeAsync(CHECK_EVERY_MS);
			expect(reg.updates).toBe(4);
		} finally {
			vi.useRealTimers();
		}
	});

	test('coming back to the app looks again once an hour has passed since the last look', async () => {
		const reg = new Registration();
		let now = 1_000_000;
		const u = new AppUpdate();
		u.start('v1', { container: container(reg), reload: vi.fn(), now: () => now, every: timer().every });
		await flush();
		expect(reg.updates).toBe(1); // the launch's own look
		u.check();
		now += CHECK_EVERY_MS - 1;
		u.check();
		expect(reg.updates).toBe(1);
		now += 1;
		u.check();
		u.check();
		expect(reg.updates).toBe(2);
	});

	test('a failing update check is quiet', async () => {
		const reg = new Registration();
		reg.update = async () => {
			throw new Error('offline');
		};
		const { t, every } = timer();
		const u = new AppUpdate();
		u.start('v1', { container: container(reg), reload: vi.fn(), every });
		await flush();
		expect(() => t.fn!()).not.toThrow();
		await flush();
	});

	test('no service worker (or a failing one): nothing offered, nothing thrown', async () => {
		const u = new AppUpdate();
		u.start('v1', { container: null });
		expect(u.offered).toBe(false);
		u.setPlaying(false);
		u.check();
		const broken = container(new Registration());
		broken.getRegistration = async () => {
			throw new Error('denied');
		};
		const v = new AppUpdate();
		v.start('v1', { container: broken });
		await flush();
		expect(v.offered).toBe(false);
		v.stop();
	});
});

describe('the status slot offers it last', () => {
	const NONE = { moving: '', reach: 'online' as const, message: '', reconnecting: false, heldBack: false, resume: false };
	test('lowest priority: anything else lasting shows first', () => {
		expect(statusOf({ ...NONE, update: true })).toEqual({ kind: 'update', text: UPDATE_TEXT });
		expect(statusOf({ ...NONE, resume: true, update: true })!.kind).toBe('resume');
		expect(statusOf({ ...NONE, reconnecting: true, update: true })!.kind).toBe('reconnecting');
	});

	test('the slot shows Reload and Later', async () => {
		const { appUpdate } = await import('#lib/pwa/update.svelte');
		const { default: StatusSlot } = await import('#lib/components/StatusSlot.svelte');
		const reg = new Registration();
		reg.waiting = new Worker();
		const c = container(reg);
		appUpdate.setPlaying(true); // only offered while this device plays
		appUpdate.start('v1', { container: c, reload: vi.fn(), every: timer().every });
		await flush();
		expect(reg.waiting.posted).toEqual([]);
		const r = render(StatusSlot);
		await tick();
		const slot = r.container.querySelector('[data-status="update"]')!;
		expect(slot.textContent).toContain(UPDATE_TEXT);
		await fireEvent.click(slot.querySelector('[data-update-reload]')!);
		expect(reg.waiting.posted).toEqual([{ type: MSG.skipWaiting }]);
		await fireEvent.click(slot.querySelector('[data-update-later]')!);
		await tick();
		expect(r.container.querySelector('[data-status="update"]')).toBeNull();
	});
});
