// A new version of the app is offered, never forced: the page keeps running what it loaded,
// shows a quiet note in the status slot, and reloads only when the person chooses.
import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { AppUpdate, CHECK_EVERY_MS, type SwContainer, type SwRegistration, type SwWorker } from '#lib/pwa/update.svelte';
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

describe('AppUpdate', () => {
	test('a waiting new version is offered; "Reload" lets it take over, then reloads', async () => {
		const reg = new Registration();
		const waiting = new Worker();
		waiting.state = 'installed';
		reg.waiting = waiting;
		const c = container(reg);
		const reload = vi.fn();
		const u = new AppUpdate();
		u.start('v1', { container: c, reload });
		await flush();
		expect(u.offered).toBe(true);
		// The page said hello, so the worker can tidy what no tab needs.
		expect(c.posted).toEqual([{ type: MSG.hello }]);
		u.apply();
		expect(waiting.posted).toEqual([{ type: MSG.skipWaiting }]);
		expect(reload).not.toHaveBeenCalled(); // not before the new version controls this page
		c.emit('controllerchange');
		expect(reload).toHaveBeenCalledOnce();
	});

	test('an update found while open is offered once installed', async () => {
		const reg = new Registration();
		const u = new AppUpdate();
		u.start('v1', { container: container(reg), reload: vi.fn() });
		await flush();
		expect(u.offered).toBe(false);
		const w = new Worker();
		reg.installing = w;
		reg.emit('updatefound');
		w.state = 'installed';
		w.emit('statechange');
		expect(u.offered).toBe(true);
	});

	test('a first install (no controller yet) is not an update', async () => {
		const reg = new Registration();
		reg.waiting = new Worker();
		const u = new AppUpdate();
		u.start('v1', { container: container(reg, false), reload: vi.fn() });
		await flush();
		expect(u.offered).toBe(false);
	});

	test('"Later" hides it until the next new version', async () => {
		const reg = new Registration();
		reg.waiting = new Worker();
		const u = new AppUpdate();
		u.start('v1', { container: container(reg), reload: vi.fn() });
		await flush();
		u.dismiss();
		expect(u.offered).toBe(false);
		const w = new Worker();
		reg.installing = w;
		reg.emit('updatefound');
		w.state = 'installed';
		w.emit('statechange');
		expect(u.offered).toBe(true);
	});

	test('the page tells the worker its version (and nothing for other messages)', async () => {
		const c = container(new Registration());
		const u = new AppUpdate();
		u.start('build-7', { container: c, reload: vi.fn() });
		const replies: unknown[] = [];
		const port = { postMessage: (m: unknown) => replies.push(m) };
		c.emit('message', { data: { type: MSG.ask }, ports: [port] });
		c.emit('message', { data: { type: 'other' }, ports: [port] });
		c.emit('message', { data: null, ports: [port] });
		c.emit('message', { data: { type: MSG.ask }, ports: [] });
		expect(replies).toEqual([{ type: MSG.reply, version: 'build-7' }]);
	});

	test('coming back to the app looks for a new version at most once an hour', async () => {
		const reg = new Registration();
		let now = 1_000_000;
		const u = new AppUpdate();
		u.start('v1', { container: container(reg), reload: vi.fn(), now: () => now });
		await flush();
		u.check();
		expect(reg.updates).toBe(0);
		now += CHECK_EVERY_MS;
		u.check();
		u.check();
		expect(reg.updates).toBe(1);
	});

	test('no service worker (or a failing one): nothing offered, nothing thrown', async () => {
		const u = new AppUpdate();
		u.start('v1', { container: null });
		expect(u.offered).toBe(false);
		const broken = container(new Registration());
		broken.getRegistration = async () => {
			throw new Error('denied');
		};
		const v = new AppUpdate();
		v.start('v1', { container: broken });
		await flush();
		expect(v.offered).toBe(false);
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
		appUpdate.start('v1', { container: c, reload: vi.fn() });
		await flush();
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
