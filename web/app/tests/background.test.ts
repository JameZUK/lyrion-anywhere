// Background playback helpers: the keep-alive element and the reload-proof log.
import { KeepAlive, PersistentLog, describeLifecycle, type MediaLike } from '#lib/player/background';

class FakeMedia implements MediaLike {
	loop = false;
	preload = '';
	paused = true;
	attrs: Record<string, string> = {};
	plays = 0;
	constructor(public src: string, private refuse = false) {}
	async play() {
		this.plays++;
		if (this.refuse) throw new DOMException('not allowed', 'NotAllowedError');
		this.paused = false;
	}
	pause() {
		this.paused = true;
	}
	setAttribute(k: string, v: string) {
		this.attrs[k] = v;
	}
}

function memoryStore() {
	const m = new Map<string, string>();
	return {
		getItem: (k: string) => m.get(k) ?? null,
		setItem: (k: string, v: string) => void m.set(k, v),
		removeItem: (k: string) => void m.delete(k),
		raw: m,
	};
}

describe('keep-alive', () => {
	test('plays a looping, inline, hidden element and stops it', async () => {
		let el!: FakeMedia;
		const k = new KeepAlive('/player/silence.wav', (s) => (el = new FakeMedia(s)));
		expect(await k.start()).toBe(true);
		expect(el.src).toBe('/player/silence.wav');
		expect(el.loop).toBe(true);
		expect(el.attrs).toMatchObject({ playsinline: '', 'aria-hidden': 'true' });
		expect(k.running).toBe(true);
		k.stop();
		expect(el.paused).toBe(true);
		expect(k.running).toBe(false);
	});

	test('a refused play (no user gesture) is reported, not thrown', async () => {
		const k = new KeepAlive('x', (s) => new FakeMedia(s, true));
		await expect(k.start()).resolves.toBe(false);
	});

	test('nudge restarts a paused element and leaves a running one alone', async () => {
		let el!: FakeMedia;
		const k = new KeepAlive('x', (s) => (el = new FakeMedia(s)));
		await k.start();
		await k.nudge();
		expect(el.plays).toBe(1);
		el.pause(); // e.g. the system paused it for a phone call
		await k.nudge();
		expect(el.plays).toBe(2);
		expect(k.running).toBe(true);
	});
});

describe('persistent log', () => {
	test('survives a reload (a new instance reads it back), newest first, bounded', () => {
		const store = memoryStore();
		const a = new PersistentLog(store);
		for (let i = 0; i < 250; i++) a.add(`line ${i}`, new Date(2026, 0, 1, 12, 0, i % 60));
		const b = new PersistentLog(store);
		expect(b.lines).toHaveLength(200);
		expect(b.lines[0]).toContain('line 249');
	});

	test('the active flag survives a reload and can be cleared', () => {
		const store = memoryStore();
		const a = new PersistentLog(store);
		expect(a.active).toBe(false);
		a.active = true;
		expect(new PersistentLog(store).active).toBe(true);
		a.active = false;
		expect(new PersistentLog(store).active).toBe(false);
	});

	test('corrupt or hostile stored data is ignored', () => {
		for (const bad of ['not json', '{"a":1}', '[1,2,{"x":"<img src=x>"}]', 'null']) {
			const store = memoryStore();
			store.setItem('anywhere.device.log', bad);
			const l = new PersistentLog(store);
			expect(l.lines.every((x) => typeof x === 'string')).toBe(true);
		}
	});

	test('blocked storage still gives a working in-memory log', () => {
		const blocked = {
			getItem: () => { throw new Error('blocked'); },
			setItem: () => { throw new Error('blocked'); },
			removeItem: () => { throw new Error('blocked'); },
		};
		const l = new PersistentLog(blocked);
		expect(l.add('hello')).toHaveLength(1);
		expect(l.active).toBe(false);
		l.active = true; // no throw
	});

	test('long messages are cut', () => {
		const l = new PersistentLog(memoryStore());
		expect(l.add('x'.repeat(5000))[0]!.length).toBeLessThan(400);
	});
});

test('lifecycle events are described for people', () => {
	expect(describeLifecycle('visibilitychange', { visibility: 'hidden' })).toMatch(/hidden/);
	expect(describeLifecycle('visibilitychange', { visibility: 'visible' })).toMatch(/visible/);
	expect(describeLifecycle('pageshow', { persisted: true })).toMatch(/restored/);
	expect(describeLifecycle('pagehide', { persisted: false })).toMatch(/closing/);
	expect(describeLifecycle('freeze')).toMatch(/frozen/);
});
