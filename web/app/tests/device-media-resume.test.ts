export {};
// Media-element mode: the element's real state, not what was asked of it, decides whether the
// sound is on. Live (iPadOS, the app reopened as a PWA): the start without a tap was refused,
// the tap that resumed it was aborted by a stream restart at the same moment, and nothing
// tried again. The element stayed paused, the "tap to resume" hold was cleared, and LMS, the
// core and the UI all carried on as if it were playing.

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
	appendBuffer() {
		this.updating = true;
		setTimeout(() => {
			this.updating = false;
			this.dispatchEvent(new Event('updateend'));
		}, 0);
	}
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

/**
 * Elements behave as Safari's do. Each play() takes the next of `results`: 'ok' plays;
 * 'NotAllowedError' is refused outright (no gesture); 'AbortError' starts (paused is false at
 * once) and is then aborted by a load or source change, leaving the element paused, with no
 * pause event.
 */
const results: string[] = [];
let plays = 0;
const paused = new WeakMap<HTMLMediaElement, boolean>();
const times = new WeakMap<HTMLMediaElement, number>();
Object.defineProperty(HTMLMediaElement.prototype, 'paused', {
	configurable: true,
	get(this: HTMLMediaElement) {
		return paused.get(this) ?? true;
	},
});
Object.defineProperty(HTMLMediaElement.prototype, 'currentTime', {
	configurable: true,
	get(this: HTMLMediaElement) {
		return times.get(this) ?? 0;
	},
	set(this: HTMLMediaElement, v: number) {
		times.set(this, v);
	},
});
HTMLMediaElement.prototype.play = async function (this: HTMLMediaElement) {
	// The background keep-alive's element (silence.wav) just plays.
	if (!this.getAttribute('src')?.startsWith('blob:')) return void paused.set(this, false);
	plays++;
	const r = results.shift() ?? 'ok';
	if (r === 'NotAllowedError') throw new DOMException('The request is not allowed by the user agent', r);
	paused.set(this, false);
	if (r === 'AbortError') {
		await Promise.resolve();
		paused.set(this, true);
		throw new DOMException('The operation was aborted.', r);
	}
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
const element = () => [...document.querySelectorAll('audio')].filter((a) => a.getAttribute('src')?.startsWith('blob:')).at(-1)!;
const tap = () => document.dispatchEvent(new Event('pointerup'));
/** Log lines added since it had `n` lines (newest first; the log keeps 200, far more than this file adds). */
const logSince = (n: number) => {
	expect(device.log.length).toBeLessThan(200);
	return device.log.slice(0, device.log.length - n);
};
const played = (w: FakeWorker) => w.posted.filter((m) => m.cmd === 'played').map((m) => m.frames as number);

/** Started in media mode (by itself when `auto`), connected, and LMS playing. */
async function started(auto: boolean) {
	localStorage.setItem('anywhere.playback.mode', 'media');
	device.selftest = { results: [{ rate: 44_100, code: 0, ok: true }, { rate: 96_000, code: 0, ok: true }] } as unknown as typeof device.selftest;
	await device.start({ auto });
	await settle();
	expect(device.choice?.mode).toBe('media');
	const w = FakeWorker.last!;
	w.say({ connection: 'connected', player: { mac: '02:4c:41:00:00:01', name: 'Test' } });
	// The page reads the worker's status at most every 500 ms.
	await new Promise((r) => setTimeout(r, 550));
	w.say({ status: { output: 'Playing' } });
	expect(device.detail.output).toBe('Playing');
	// The first audio: the timeline starts at 0, 44.1 kHz.
	w.say({ media: { span: { at: 0, rate: 44_100 }, init: new Uint8Array([0]) } });
	await settle();
	return w;
}

beforeEach(() => {
	results.length = 0;
	plays = 0;
});
afterEach(() => {
	device.stop();
	localStorage.clear();
});

// (b) The field report, end to end.
test('a tap aborted by a stream restart plays once the new audio is in; the hold lasts until then', async () => {
	results.push('NotAllowedError'); // the start without a tap
	const w = await started(true);
	expect(device.needsTap).toBe(true);
	expect(element().paused).toBe(true);
	const before = device.log.length;
	results.push('AbortError', 'ok');
	tap();
	await settle();
	// The tap's play() was aborted: the sound is not on, so the hold stays.
	expect(element().paused).toBe(true);
	expect(device.needsTap).toBe(true);
	expect(logSince(before).some((l) => /audio resumed/.test(l))).toBe(false);
	// The restart: a flush, then the new audio from where it starts.
	w.say({ media: { flush: true } });
	await settle();
	w.say({ media: { span: { at: 0, rate: 44_100 }, init: new Uint8Array([1]), segment: new Uint8Array([2]) } });
	await settle();
	expect(element().paused).toBe(false);
	expect(device.needsTap).toBe(false);
	const log = logSince(before);
	expect(log.some((l) => /retrying play \(1\)/.test(l))).toBe(true);
	expect(log.filter((l) => /audio resumed \(media\)/.test(l))).toHaveLength(1);
});

// (c) A play refused for want of a gesture goes back to "tap to resume", and nothing counts
// as played meanwhile.
test('a refused play() puts the hold back, claims nothing, and reports no frames', async () => {
	const w = await started(false);
	expect(element().paused).toBe(false);
	expect(device.needsTap).toBe(false);
	// The system paused the element; the worker's play comes when no gesture is in hand.
	element().pause();
	results.push('NotAllowedError');
	const before = device.log.length;
	w.say({ media: { play: true } });
	await settle();
	expect(device.needsTap).toBe(true);
	expect(logSince(before).some((l) => /tap anywhere to resume/.test(l))).toBe(true);
	expect(logSince(before).some((l) => /audio resumed/.test(l))).toBe(false);
	// Safari moves the play head of a paused element (e.g. onto the buffered audio): not heard.
	const from = w.posted.length;
	element().currentTime = 1.5;
	await new Promise((r) => setTimeout(r, 350));
	const frames = w.posted.slice(from).filter((m) => m.cmd === 'played').map((m) => m.frames);
	expect(frames.length).toBeGreaterThan(0);
	expect(frames.every((f) => f === 0)).toBe(true);
	// The tap: plays, and only now is the sound claimed back.
	tap();
	await settle();
	expect(element().paused).toBe(false);
	expect(device.needsTap).toBe(false);
	expect(logSince(before).filter((l) => /audio resumed \(media\)/.test(l))).toHaveLength(1);
});

// (d) Only an element that is really playing reports progress.
test('a paused element reports no progress; a playing one does', async () => {
	const w = await started(false);
	const a = element();
	let from = w.posted.length;
	a.currentTime = 1;
	await new Promise((r) => setTimeout(r, 250));
	expect(played(w).slice(0).reduce((s, f) => s + f, 0)).toBeGreaterThan(0); // playing: counted
	w.say({ media: { pause: true } });
	await new Promise((r) => setTimeout(r, 250)); // the reports up to the pause
	from = w.posted.length;
	a.currentTime = 3; // paused, yet the play head moved
	await new Promise((r) => setTimeout(r, 350));
	const after = w.posted.slice(from).filter((m) => m.cmd === 'played').map((m) => m.frames as number);
	expect(after.length).toBeGreaterThan(0);
	expect(after.every((f) => f === 0)).toBe(true);
	// Playing again from there: counted from where it is, not from before the move.
	w.say({ media: { play: true } });
	await settle();
	from = w.posted.length;
	a.currentTime = 3.5;
	await new Promise((r) => setTimeout(r, 250));
	const resumed = w.posted.slice(from).filter((m) => m.cmd === 'played').reduce((s, m) => s + (m.frames as number), 0);
	expect(resumed).toBeGreaterThan(0);
	expect(resumed).toBeLessThanOrEqual(Math.round(0.5 * 44_100) + 2_000);
});

// 3. Where the browser is known to want a gesture, an automatic start shows the hold at
// once and leaves the element alone until the tap, rather than trying and failing.
test('an automatic start on a browser that refused before waits for the tap', async () => {
	localStorage.setItem('anywhere.playback.gesture', 'needed');
	const starting = device.start({ auto: true });
	expect(device.needsTap).toBe(true);
	await starting;
	await settle();
	expect(plays).toBe(0);
	expect(element().paused).toBe(true);
	const w = FakeWorker.last!;
	w.say({ connection: 'connected', player: { mac: '02:4c:41:00:00:01', name: 'Test' } });
	await new Promise((r) => setTimeout(r, 550));
	w.say({ status: { output: 'Playing' } });
	w.say({ media: { play: true } }); // the worker's play: still no gesture, so not tried
	await settle();
	expect(plays).toBe(0);
	tap();
	await settle();
	expect(plays).toBe(1);
	expect(element().paused).toBe(false);
	expect(device.needsTap).toBe(false);
});

test('a refusal of an automatic start is remembered; browsers that allow it start at once', async () => {
	await started(true); // allowed: plays without a tap
	expect(plays).toBe(1);
	expect(device.needsTap).toBe(false);
	expect(localStorage.getItem('anywhere.playback.gesture')).toBeNull();
	device.stop();
	results.push('NotAllowedError');
	await started(true);
	expect(device.needsTap).toBe(true);
	expect(localStorage.getItem('anywhere.playback.gesture')).toBe('needed');
});

test('with a gesture in hand, an automatic start tries even where it was refused before', async () => {
	localStorage.setItem('anywhere.playback.gesture', 'needed');
	Object.defineProperty(navigator, 'userActivation', { configurable: true, value: { isActive: true, hasBeenActive: true } });
	try {
		await started(true);
		expect(plays).toBe(1);
		expect(device.needsTap).toBe(false);
		expect(localStorage.getItem('anywhere.playback.gesture')).toBeNull();
	} finally {
		delete (navigator as { userActivation?: unknown }).userActivation;
	}
});
