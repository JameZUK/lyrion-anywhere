export {};
// Protocol 2 on the page (docs/plans/signal-path-redesign.md): the streaming level overridden
// on the signal path (applies now with a seek, else from the next track; cleared on a reload
// and on a change of network type), the gateway's 4426 (a stale page: never restarted), and
// the /v1/me protocol gate. Against a fake worker and a fake Web Audio stack, as device.test.ts.
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
const api = vi.hoisted(() => ({ calls: [] as { player: string; cmd: unknown[] }[], async rpc(player: string, cmd: unknown[]) {
	api.calls.push({ player, cmd });
	return {};
} }));
vi.mock('#lib/state/app.svelte', () => ({ session, api }));

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
/** False: a context without `audioWorklet`, as on a plain-http page. */
let worklets = true;
/** Like a browser without a user gesture yet: contexts start suspended until one. */
let gesture = { needed: false, given: false };
class FakeAudioContext {
	static all: FakeAudioContext[] = [];
	state: 'running' | 'suspended' | 'closed' = gesture.needed ? 'suspended' : 'running';
	sampleRate: number;
	destination = {};
	// On the prototype, as in a browser (capability detection looks there).
	get audioWorklet() {
		return worklets ? { addModule: async () => addModuleGate ?? undefined } : undefined;
	}
	constructor(o: { sampleRate: number }) {
		this.sampleRate = o.sampleRate;
		FakeAudioContext.all.push(this);
	}
	async resume() {
		if (this.state === 'suspended' && (!gesture.needed || gesture.given)) this.state = 'running';
	}
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

const { device, QUALITY_ECHO_WAIT_MS } = await import('#lib/player/device.svelte');

const settle = async () => {
	for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const open = () => FakeAudioContext.all.filter((c) => c.state !== 'closed');

beforeEach(() => {
	device.stop();
	FakeWorker.all = [];
	FakeAudioContext.all = [];
	addModuleGate = null;
	worklets = true;
	gesture = { needed: false, given: false };
	session.current = { access_token: 't1' };
	session.token = 't1';
	session.renewResult = true;
	session.renewCalls = 0;
	localStorage.clear();
	api.calls.length = 0;
	device.outdated = false;
	device.serverTooOld = false;
	device.protocolGate = null;
	device.qualityOverride = null;
	device.applyStatus({});
});

async function started() {
	await device.start();
	const w = FakeWorker.all.at(-1)!;
	expect(w).toBeDefined();
	w.say({ connection: 'connected', player: { mac: '02:4c:41:00:00:01', name: 'Test' } });
	expect(device.state).toBe('connected');
	return w;
}


const MAC = '02:4c:41:00:00:01';
const seeks = () => api.calls.filter((c) => c.cmd[0] === 'time');

function mobile() {
	const conn = Object.assign(new EventTarget(), { type: 'cellular', saveData: false });
	Object.defineProperty(navigator, 'connection', { value: conn, configurable: true });
	return conn;
}

describe('the streaming level overridden on the signal path', () => {
	afterEach(() => {
		device.setQualityPreference('auto');
		Object.defineProperty(navigator, 'connection', { value: undefined, configurable: true });
	});

	test('applies now: the level is sent, and once the gateway confirms it the track is sought to where it is', async () => {
		mobile();
		device.setQualityPreference('auto');
		const w = await started();
		expect(w.cmds('start')[0]!.quality).toBe('high');
		expect(device.overrideQuality('original', { seekable: true, seconds: 42.04 })).toBe('now');
		expect(device.qualityOverride).toBe('original');
		expect(device.quality).toBe('original');
		expect(w.cmds('quality').at(-1)).toEqual({ cmd: 'quality', quality: 'original' });
		// Not before the gateway has applied the level: the new stream would still be at the old one.
		expect(seeks()).toEqual([]);
		w.say({ qualityApplied: 'original' });
		expect(seeks()).toEqual([{ player: MAC, cmd: ['time', 42] }]);
		// The setting is not changed: the override lives in memory only.
		expect(localStorage.getItem('anywhere.stream.quality')).toBe('auto');
		expect(device.qualityPreference).toBe('auto');
		// Without a position given, the engine's own is used.
		device.applyStatus({ output: 'Paused', elapsed_ms: 61_500 });
		expect(device.overrideQuality('low', { seekable: true })).toBe('now');
		w.say({ qualityApplied: 'low' });
		expect(seeks().at(-1)).toEqual({ player: MAC, cmd: ['time', 61.5] });
		// A later echo of a level already applied seeks nothing more.
		w.say({ qualityApplied: 'low' });
		expect(seeks()).toHaveLength(2);
	});

	test('no echo from the gateway: the stream restarts after the wait anyway, and the log says so', async () => {
		mobile();
		const w = await started();
		vi.useFakeTimers();
		try {
			expect(device.overrideQuality('original', { seekable: true, seconds: 10 })).toBe('now');
			vi.advanceTimersByTime(QUALITY_ECHO_WAIT_MS - 1);
			expect(seeks()).toEqual([]);
			vi.advanceTimersByTime(1);
			expect(seeks()).toEqual([{ player: MAC, cmd: ['time', 10] }]);
			expect(device.log.some((l) => /didn't confirm the streaming level \(Original\) within 2 s/.test(l))).toBe(true);
			// An echo arriving late does not seek a second time.
			w.say({ qualityApplied: 'original' });
			vi.advanceTimersByTime(QUALITY_ECHO_WAIT_MS * 2);
			expect(seeks()).toHaveLength(1);
		} finally {
			vi.useRealTimers();
		}
	});

	test("junk, and an echo of another level, don't count as the gateway's confirmation", async () => {
		mobile();
		const w = await started();
		vi.useFakeTimers();
		try {
			device.overrideQuality('original', { seekable: true, seconds: 7 });
			for (const junk of ['ultra', 'ORIGINAL', '', 1, null, ['original'], { quality: 'original' }, 'high']) w.say({ qualityApplied: junk });
			expect(seeks()).toEqual([]);
			vi.advanceTimersByTime(QUALITY_ECHO_WAIT_MS);
			expect(seeks()).toEqual([{ player: MAC, cmd: ['time', 7] }]);
		} finally {
			vi.useRealTimers();
		}
	});

	test('a newer choice supersedes a restart still waiting for its echo; stopping cancels it', async () => {
		mobile();
		const w = await started();
		vi.useFakeTimers();
		try {
			device.overrideQuality('original', { seekable: true, seconds: 5 });
			device.overrideQuality('low', { seekable: true, seconds: 6 });
			w.say({ qualityApplied: 'original' });
			expect(seeks()).toEqual([]);
			w.say({ qualityApplied: 'low' });
			expect(seeks()).toEqual([{ player: MAC, cmd: ['time', 6] }]);
			device.overrideQuality('original', { seekable: true, seconds: 8 });
			device.stop();
			vi.advanceTimersByTime(QUALITY_ECHO_WAIT_MS * 2);
			expect(seeks()).toHaveLength(1);
		} finally {
			vi.useRealTimers();
		}
	});

	test('decided by the stream being sent: after a change in Settings mid-track, the level now chosen there still restarts the stream', async () => {
		mobile();
		const w = await started();
		// High on mobile data: the stream in progress is Opus 160.
		device.applyStatus({ output: 'Playing', elapsed_ms: 20_000, sent: { mode: 'opus', kbps: 160, level: 'high' } });
		// Settings › Original: applies from the next track; the gateway confirms the level.
		device.setQualityPreference('original');
		expect(device.quality).toBe('original');
		w.say({ qualityApplied: 'original' });
		expect(seeks()).toEqual([]);
		// A tap on Original on the signal path: what is heard is High, so it applies now.
		expect(device.overrideQuality('original', { seekable: true, seconds: 20 })).toBe('now');
		expect(seeks()).toEqual([{ player: MAC, cmd: ['time', 20] }]);
	});

	test('the worker already holds the level but the gateway never confirmed it: it is asked again, and the echo seeks', async () => {
		mobile();
		const w = await started();
		device.applyStatus({ output: 'Paused', elapsed_ms: 20_000, sent: { mode: 'opus', kbps: 160, level: 'high' } });
		device.setQualityPreference('original');
		const n = w.cmds('quality').length;
		expect(device.overrideQuality('original', { seekable: true, seconds: 20 })).toBe('now');
		expect(w.cmds('quality').slice(n)).toEqual([{ cmd: 'quality', quality: 'original', confirm: true }]);
		expect(seeks()).toEqual([]);
		w.say({ qualityApplied: 'original' });
		expect(seeks()).toEqual([{ player: MAC, cmd: ['time', 20] }]);
	});

	test('the stream being sent is already at the chosen level: unchanged, nothing sought', async () => {
		mobile();
		await started();
		// Settings › High mid-track, while the stream in progress is still passed through.
		device.applyStatus({ output: 'Playing', elapsed_ms: 5_000, sent: { mode: 'passthrough' } });
		expect(device.overrideQuality('original', { seekable: true, seconds: 5 })).toBe('unchanged');
		expect(device.quality).toBe('original');
		expect(device.overrideQuality('low', { seekable: true, seconds: 5 })).toBe('now');
		device.applyStatus({ output: 'Playing', elapsed_ms: 6_000, sent: { mode: 'opus', kbps: 96, level: 'low' } });
		expect(device.overrideQuality('low', { seekable: true, seconds: 6 })).toBe('unchanged');
	});

	test('an unseekable stream (radio): applies from the next track, no seek', async () => {
		mobile();
		const w = await started();
		expect(device.overrideQuality('low', { seekable: false })).toBe('next');
		expect(w.cmds('quality').at(-1)).toEqual({ cmd: 'quality', quality: 'low' });
		expect(seeks()).toEqual([]);
	});

	test('choosing the level already in force changes nothing and seeks nothing', async () => {
		mobile();
		const w = await started();
		const n = w.cmds('quality').length;
		expect(device.overrideQuality('high', { seekable: true, seconds: 10 })).toBe('unchanged');
		expect(w.cmds('quality').length).toBe(n);
		expect(seeks()).toEqual([]);
		// Still an override: the switch says "overridden here", and offers Back to automatic.
		expect(device.qualityOverride).toBe('high');
	});

	test('back to automatic: the per-network level applies again', async () => {
		mobile();
		const w = await started();
		device.overrideQuality('original', { seekable: true, seconds: 5 });
		expect(device.overrideQuality(null, { seekable: true, seconds: 6 })).toBe('now');
		expect(device.qualityOverride).toBeNull();
		expect(w.cmds('quality').at(-1)).toEqual({ cmd: 'quality', quality: 'high' });
		w.say({ qualityApplied: 'high' });
		expect(seeks()).toHaveLength(1);
		expect(seeks().at(-1)!.cmd).toEqual(['time', 6]);
	});

	test('cleared when the network type changes, not on other connection changes', async () => {
		const conn = mobile();
		const w = await started();
		device.overrideQuality('original', { seekable: false });
		// A change of speed only: the override stays.
		conn.dispatchEvent(new Event('change'));
		expect(device.qualityOverride).toBe('original');
		expect(device.quality).toBe('original');
		// Mobile data to Wi-Fi: back to the per-network choice (Original on Wi-Fi), override gone.
		conn.type = 'wifi';
		conn.dispatchEvent(new Event('change'));
		expect(device.qualityOverride).toBeNull();
		expect(device.log.some((l) => l.includes('override ended'))).toBe(true);
		// And back to mobile data: High again, as the setting says.
		device.overrideQuality('low', { seekable: false });
		conn.type = 'cellular';
		conn.dispatchEvent(new Event('change'));
		expect(device.qualityOverride).toBeNull();
		expect(w.cmds('quality').at(-1)).toEqual({ cmd: 'quality', quality: 'high' });
	});

	test('a choice in Settings ends the override', async () => {
		mobile();
		await started();
		device.overrideQuality('original', { seekable: false });
		device.setQualityPreference('low');
		expect(device.qualityOverride).toBeNull();
		expect(device.quality).toBe('low');
	});

	test('Safari (no Network Information API): the override works, and nothing clears it but a reload', async () => {
		Object.defineProperty(navigator, 'connection', { value: undefined, configurable: true });
		device.setQualityPreference('auto');
		const w = await started();
		expect(w.cmds('start')[0]!.quality).toBe('original');
		expect(device.overrideQuality('low', { seekable: true, seconds: 3 })).toBe('now');
		expect(w.cmds('quality').at(-1)).toEqual({ cmd: 'quality', quality: 'low' });
		expect(device.qualityOverride).toBe('low');
	});

	test('not playing here: the level is kept for the next start, nothing is sought', () => {
		mobile();
		expect(device.overrideQuality('low', { seekable: true, seconds: 9 })).toBe('next');
		expect(seeks()).toEqual([]);
	});

	test('cleared on page load: a fresh page has no override, whatever this one had', async () => {
		mobile();
		device.overrideQuality('low', { seekable: false });
		vi.resetModules();
		const fresh = (await import('#lib/player/device.svelte')).device;
		expect(fresh.qualityOverride).toBeNull();
		expect(fresh.qualityPreference).toBe('auto');
	});
});

describe('close 4426: a stale page', () => {
	test('the device stops, says to reload, and never restarts by itself', async () => {
		const w = await started();
		w.say({ connection: 'closed', code: 4426, reason: 'update the app' });
		expect(w.terminated).toBe(true);
		expect(device.outdated).toBe(true);
		expect(device.state).toBe('stopped');
		expect(device.log.some((l) => /reload the page/.test(l))).toBe(true);
		// Neither a tap nor the automatic start brings it back on this page.
		await device.start();
		expect(FakeWorker.all).toHaveLength(1);
		expect(await device.autoStart()).toBe('outdated');
		expect(FakeWorker.all).toHaveLength(1);
	});

	test('any other close is handled as before (the device can start again)', async () => {
		const w = await started();
		w.say({ connection: 'closed' });
		expect(device.outdated).toBe(false);
		await device.start();
		expect(FakeWorker.all).toHaveLength(2);
	});
});

describe('the gateway protocol gate (/v1/me)', () => {
	test.each([
		[1, false],
		[null, false],
		[2, true],
		[3, true],
		[undefined, true],
	] as const)('protocol %s: started %s', async (protocol, starts) => {
		device.protocolGate = async () => protocol;
		await device.start();
		await settle();
		expect(FakeWorker.all.length > 0).toBe(starts);
		expect(device.serverTooOld).toBe(!starts);
		if (!starts) {
			// Nothing left running, and no audio left open.
			expect(device.state).toBe('off');
			expect(open()).toHaveLength(0);
			expect(device.log.some((l) => /update the server's plugin/.test(l))).toBe(true);
			expect(await device.autoStart()).toBe('server-too-old');
			expect(FakeWorker.all).toHaveLength(0);
		}
	});
});
