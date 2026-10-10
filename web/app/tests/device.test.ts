export {};
// "This device" state machine, against a fake worker and a fake Web Audio stack.
type Listener = (s: unknown, status: string) => void;
const session = {
	current: { access_token: 't1' } as unknown,
	token: 't1',
	renewResult: true,
	renewCalls: 0,
	listeners: new Set<Listener>(),
	subscribe(fn: Listener) {
		this.listeners.add(fn);
		fn(this.current, 'signed-in');
		return () => this.listeners.delete(fn);
	},
	async renew() {
		this.renewCalls++;
		if (this.renewResult) this.token = 't2';
		return this.renewResult;
	},
	async authed() {
		/* diagnostics: not under test here */
	},
};
vi.mock('$lib/state/app.svelte', () => ({ session }));

class FakeWorker {
	static all: FakeWorker[] = [];
	posted: Record<string, unknown>[] = [];
	terminated = false;
	onmessage: ((e: { data: unknown }) => void) | null = null;
	onerror: ((e: { message: string }) => void) | null = null;
	constructor(public url: string) {
		FakeWorker.all.push(this);
	}
	postMessage(m: Record<string, unknown>) {
		this.posted.push(m);
	}
	terminate() {
		this.terminated = true;
	}
	/** A message from the worker to the page. */
	say(data: Record<string, unknown>) {
		this.onmessage?.({ data });
	}
	cmds(cmd: string) {
		return this.posted.filter((m) => m.cmd === cmd);
	}
}

let addModuleGate: Promise<void> | null = null;
class FakeAudioContext {
	static all: FakeAudioContext[] = [];
	state: 'running' | 'suspended' | 'closed' = 'running';
	sampleRate: number;
	destination = {};
	audioWorklet = { addModule: async () => addModuleGate ?? undefined };
	constructor(o: { sampleRate: number }) {
		this.sampleRate = o.sampleRate;
		FakeAudioContext.all.push(this);
	}
	async resume() {}
	async close() {
		if (this.state === 'closed') throw new DOMException('already closed', 'InvalidStateError');
		this.state = 'closed';
	}
}
class FakeNode {
	port = { postMessage() {} };
	connect() {}
}
class FakePort {
	closed = false;
	close() {
		this.closed = true;
	}
}
class FakeChannel {
	port1 = new FakePort();
	port2 = new FakePort();
}

vi.stubGlobal('Worker', FakeWorker);
vi.stubGlobal('AudioContext', FakeAudioContext);
vi.stubGlobal('AudioWorkletNode', FakeNode);
vi.stubGlobal('MessageChannel', FakeChannel);
HTMLMediaElement.prototype.play = async () => {};
HTMLMediaElement.prototype.pause = () => {};
HTMLMediaElement.prototype.load = () => {};

const { device } = await import('$lib/player/device.svelte');

const settle = async () => {
	for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const open = () => FakeAudioContext.all.filter((c) => c.state !== 'closed');

beforeEach(() => {
	device.stop();
	FakeWorker.all = [];
	FakeAudioContext.all = [];
	addModuleGate = null;
	session.current = { access_token: 't1' };
	session.token = 't1';
	session.renewResult = true;
	session.renewCalls = 0;
	localStorage.clear();
});

async function started() {
	await device.start();
	const w = FakeWorker.all.at(-1)!;
	expect(w).toBeDefined();
	w.say({ connection: 'connected', player: { mac: '02:4c:41:00:00:01', name: 'Test' } });
	expect(device.state).toBe('connected');
	return w;
}

test('starts in Web Audio mode with one context and hands its port to the worker', async () => {
	const w = await started();
	expect(open()).toHaveLength(1);
	expect(open()[0]!.sampleRate).toBe(44100);
	expect(w.cmds('start')).toHaveLength(1);
	expect(w.cmds('rate-ready')).toEqual([{ cmd: 'rate-ready', rate: 44100 }]);
});

// Regression: the worker repeated {rate} every 20 ms until the page confirmed, and the page
// built a new AudioContext for each message, concurrently, orphaning all but the last.
test('a flood of rate requests builds one context per rate change and closes the old one', async () => {
	const w = await started();
	// As the worker used to: one request per tick, until the page confirmed the rate.
	for (let i = 0; i < 50; i++) {
		w.say({ rate: 96000 });
		await new Promise((r) => setTimeout(r, 0));
	}
	await settle();
	expect(FakeAudioContext.all.map((c) => c.sampleRate)).toEqual([44100, 96000]);
	expect(open().map((c) => c.sampleRate)).toEqual([96000]);
	expect(w.cmds('start-sink')).toHaveLength(1);
	expect(w.cmds('rate-ready').at(-1)).toEqual({ cmd: 'rate-ready', rate: 96000 });

	for (let i = 0; i < 50; i++) w.say({ rate: 48000 });
	await settle();
	expect(FakeAudioContext.all.map((c) => c.sampleRate)).toEqual([44100, 96000, 48000]);
	expect(open().map((c) => c.sampleRate)).toEqual([48000]);
	expect(w.cmds('start-sink')).toHaveLength(2);
});

test('rate requests are built one at a time; an overtaken one is dropped', async () => {
	const w = await started();
	let release!: () => void;
	addModuleGate = new Promise((r) => (release = r));
	w.say({ rate: 96000 });
	w.say({ rate: 48000 });
	w.say({ rate: 48000 });
	await settle();
	expect(FakeAudioContext.all).toHaveLength(2); // the 96 kHz one, still loading
	release();
	addModuleGate = null;
	await settle();
	expect(open().map((c) => c.sampleRate)).toEqual([48000]);
	expect(w.cmds('rate-ready').map((m) => m.rate)).toEqual([44100, 48000]);
});

test('stopping closes the context, and a late rate request builds nothing', async () => {
	const w = await started();
	device.stop();
	expect(open()).toHaveLength(0);
	w.say({ rate: 96000 });
	await settle();
	expect(FakeAudioContext.all).toHaveLength(1);
});

// Regression: after the gateway closed the player, or the worker died, the worker stayed
// set, so "Play on this device" silently did nothing.
test('a closed connection releases the worker, so the device can start again', async () => {
	const w = await started();
	w.say({ connection: 'closed' });
	expect(device.state).toBe('stopped');
	expect(w.terminated).toBe(true);
	expect(open()).toHaveLength(0);
	await device.start();
	expect(FakeWorker.all).toHaveLength(2);
	expect(device.state).toBe('starting');
});

test('a worker error shows the error and lets the device start again', async () => {
	const w = await started();
	w.onerror!({ message: 'script failed to load' });
	expect(device.state).toBe('error');
	expect(device.log.some((l) => l.includes('script failed to load'))).toBe(true);
	expect(w.terminated).toBe(true);
	await device.start();
	expect(FakeWorker.all).toHaveLength(2);
});

// Regression: pressing Stop while "Starting…" let the start carry on and create a worker.
test('stop while starting tears down what was built and starts nothing more', async () => {
	let release!: () => void;
	addModuleGate = new Promise((r) => (release = r));
	const starting = device.start();
	await settle();
	expect(device.state).toBe('starting');
	device.stop();
	expect(device.state).toBe('stopped');
	release();
	await starting;
	await settle();
	expect(FakeWorker.all).toHaveLength(0);
	expect(open()).toHaveLength(0);
	expect(device.state).toBe('stopped');
});

test('a second start while starting is ignored', async () => {
	let release!: () => void;
	addModuleGate = new Promise((r) => (release = r));
	const a = device.start();
	const b = device.start();
	release();
	await Promise.all([a, b]);
	expect(FakeWorker.all).toHaveLength(1);
	expect(FakeAudioContext.all).toHaveLength(1);
});

test('a refused token is renewed and sent back with reconnect', async () => {
	const w = await started();
	w.say({ connection: 'reauthenticating', authFailed: true });
	await settle();
	expect(session.renewCalls).toBe(1);
	expect(w.cmds('token').at(-1)).toEqual({ cmd: 'token', token: 't2', reconnect: true });
	expect(device.state).toBe('reconnecting');
});

test('a renewal that fails offline leaves the worker waiting; a later token reaches it', async () => {
	const w = await started();
	session.renewResult = false;
	w.say({ connection: 'reauthenticating', authFailed: true });
	await settle();
	expect(w.cmds('token').filter((m) => m.reconnect)).toHaveLength(0);
	// The session's own retry succeeds later and notifies its subscribers.
	for (const fn of session.listeners) fn({ access_token: 't3' }, 'signed-in');
	expect(w.cmds('token').at(-1)).toMatchObject({ token: 't3' });
	expect(w.terminated).toBe(false);
});

test('signing out stops the device', async () => {
	const w = await started();
	for (const fn of [...session.listeners]) fn(null, 'signed-out');
	expect(w.terminated).toBe(true);
	expect(device.state).toBe('stopped');
});

test('nothing starts when signed out', async () => {
	session.current = null;
	await device.start();
	expect(FakeWorker.all).toHaveLength(0);
});
