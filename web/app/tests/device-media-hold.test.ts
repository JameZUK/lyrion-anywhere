export {};
// Media-element mode: a pause made on this device while the server is out of reach holds the
// element paused. The page's own recovery (back from the background, the first tap that
// unmutes, a synced start) must never play it again behind the user's back; only the worker
// letting the hold go does.
import { mayResumeMedia } from '$lib/player/local-plan';

const session = {
	current: { access_token: 't1' } as unknown,
	token: 't1',
	subscribe(fn: (s: unknown, st: string) => void) {
		fn(this.current, 'signed-in');
		return () => {};
	},
	async renew() {
		return true;
	},
	async authed() {},
};
vi.mock('$lib/state/app.svelte', () => ({ session, api: { rpc: async () => ({}) } }));

class FakeWorker {
	static last: FakeWorker | null = null;
	posted: Record<string, unknown>[] = [];
	onmessage: ((e: { data: unknown }) => void) | null = null;
	onerror: unknown = null;
	constructor() {
		FakeWorker.last = this;
	}
	postMessage(m: Record<string, unknown>) {
		this.posted.push(m);
	}
	terminate() {}
	say(data: Record<string, unknown>) {
		this.onmessage?.({ data });
	}
}
class FakeBuffered {
	length = 0;
	start() {
		return 0;
	}
	end() {
		return 0;
	}
}
class FakeSourceBuffer extends EventTarget {
	mode = '';
	updating = false;
	buffered = new FakeBuffered();
	appendBuffer() {}
	remove() {}
	abort() {}
}
class FakeMediaSource extends EventTarget {
	static isTypeSupported = () => true;
	constructor() {
		super();
		setTimeout(() => this.dispatchEvent(new Event('sourceopen')), 0);
	}
	addSourceBuffer() {
		return new FakeSourceBuffer();
	}
}

// An element whose paused state follows play() and pause(), as a browser's does.
const paused = new WeakMap<HTMLMediaElement, boolean>();
Object.defineProperty(HTMLMediaElement.prototype, 'paused', { configurable: true, get(this: HTMLMediaElement) {
	return paused.get(this) ?? true;
} });
HTMLMediaElement.prototype.play = async function (this: HTMLMediaElement) {
	paused.set(this, false);
};
HTMLMediaElement.prototype.pause = function (this: HTMLMediaElement) {
	paused.set(this, true);
};
HTMLMediaElement.prototype.load = () => {};

vi.stubGlobal('Worker', FakeWorker);
vi.stubGlobal('MediaSource', FakeMediaSource);
URL.createObjectURL = () => 'blob:ms';
URL.revokeObjectURL = () => {};

const { device } = await import('$lib/player/device.svelte');

const settle = async () => {
	for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const element = () => [...document.querySelectorAll('audio')].at(-1)!;
const visible = () => document.dispatchEvent(new Event('visibilitychange'));

/** Started in media mode, connected, and playing as far as LMS goes. */
async function playing() {
	localStorage.setItem('anywhere.playback.mode', 'media');
	device.selftest = { results: [{ rate: 44_100, code: 0, ok: true }, { rate: 96_000, code: 0, ok: true }] } as unknown as typeof device.selftest;
	await device.start();
	await settle();
	expect(device.choice?.mode).toBe('media');
	const w = FakeWorker.last!;
	w.say({ connection: 'connected', player: { mac: '02:4c:41:00:00:01', name: 'Test' } });
	// The page reads the worker's status at most every 500 ms.
	await new Promise((r) => setTimeout(r, 550));
	w.say({ status: { output: 'Playing' } });
	expect(device.detail.output).toBe('Playing');
	expect(element().paused).toBe(false);
	return w;
}

afterEach(() => {
	device.stop();
	localStorage.clear();
});

test('the guard: only while LMS plays, never while held here', () => {
	expect(mayResumeMedia('Playing', false)).toBe(true);
	expect(mayResumeMedia('Playing', true)).toBe(false);
	expect(mayResumeMedia('Held', false)).toBe(false);
	expect(mayResumeMedia('Paused { until: None }', false)).toBe(false);
	for (const v of [undefined, null, 1, {}]) expect(mayResumeMedia(v, false)).toBe(false);
});

test('back from the background: a paused-by-the-system element resumes (the path works)…', async () => {
	await playing();
	paused.set(element(), true); // the system paused it
	visible();
	await settle();
	expect(element().paused).toBe(false);
});

test('…but one held by a pause made here stays paused', async () => {
	const w = await playing();
	device.local({ hold: true });
	expect(w.posted.at(-1)).toEqual({ cmd: 'local', hold: true });
	expect(element().paused).toBe(true);
	// The core still says Playing (the hold is the worker's, not LMS's).
	w.say({ status: { output: 'Playing', held: true } });
	visible();
	await settle();
	expect(element().paused).toBe(true);
	addEventListener('pageshow', () => {});
	window.dispatchEvent(Object.assign(new Event('pageshow'), { persisted: true }));
	await settle();
	expect(element().paused).toBe(true);
	// A synced start that comes meanwhile doesn't play it either.
	w.say({ media: { startAt: performance.timeOrigin + performance.now() + 5 } });
	await new Promise((r) => setTimeout(r, 30));
	expect(element().paused).toBe(true);
	// Let go here: the worker says when to play (its play message), and then it does.
	device.local({ release: 'now' });
	w.say({ media: { play: true } });
	expect(element().paused).toBe(false);
});
