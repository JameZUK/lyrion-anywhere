export {};
// "Fix this player's output level at 100%": this device keeps its own player's Volume Control
// in LMS (`digitalVolumeControl`) in step with the setting, against a fake worker, a fake Web
// Audio stack and a fake LMS behind `api.rpc`.
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

type Cmd = (string | number)[];
/** The fake LMS: what this player's Volume Control reads, and every command sent. */
const lms = {
	control: 1 as number | undefined,
	sent: [] as { player: string; cmd: Cmd }[],
	failStatus: false,
	failPref: false,
};
const api = {
	rpc: vi.fn(async (player: string, cmd: Cmd) => {
		lms.sent.push({ player, cmd });
		if (cmd[0] === 'status') {
			if (lms.failStatus) throw new Error('LMS is busy');
			return lms.control === undefined ? { mode: 'stop' } : { mode: 'stop', digital_volume_control: lms.control };
		}
		if (cmd[0] === 'playerpref') {
			if (lms.failPref) throw new Error('refused');
			lms.control = Number(cmd[2]);
		}
		return {};
	}),
};
vi.mock('#lib/state/app.svelte', () => ({ session, api }));

class FakeWorker {
	static last: FakeWorker | null = null;
	onmessage: ((e: { data: unknown }) => void) | null = null;
	onerror: unknown = null;
	constructor() {
		FakeWorker.last = this;
	}
	postMessage() {}
	terminate() {}
	say(data: Record<string, unknown>) {
		this.onmessage?.({ data });
	}
}
class FakeAudioContext {
	state = 'running';
	sampleRate: number;
	destination = {};
	get audioWorklet() {
		return { addModule: async () => undefined };
	}
	constructor(o: { sampleRate: number }) {
		this.sampleRate = o.sampleRate;
	}
	async resume() {}
	async close() {
		this.state = 'closed';
	}
}
class FakeNode {
	port = { postMessage() {} };
	connect() {}
}
class FakePort {
	close() {}
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

// A clock the tests move by hand (the re-assert is rate-limited on performance.now()).
let clock = 1_000_000;
vi.spyOn(performance, 'now').mockImplementation(() => clock);

const { device, FIXED_LEVEL_EVERY_MS } = await import('#lib/player/device.svelte');

const MAC = '02:4c:41:00:00:01';
const settle = async () => {
	for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const prefs = () => lms.sent.filter((s) => s.cmd[0] === 'playerpref');

async function connected() {
	await device.start();
	FakeWorker.last!.say({ connection: 'connected', player: { mac: MAC, name: 'Test' } });
	expect(device.state).toBe('connected');
	await settle();
}

beforeEach(() => {
	device.stop();
	device.mac = '';
	localStorage.clear();
	device.fixedLevel = true;
	lms.control = 1;
	lms.sent = [];
	lms.failStatus = false;
	lms.failPref = false;
	clock += 10 * FIXED_LEVEL_EVERY_MS;
});
afterEach(() => {
	device.stop();
	device.onLevelChanged = null;
});

test('the setting is on by default', () => {
	expect(device.fixedLevel).toBe(true);
});

test('on connecting, fixes the level once when LMS lets it set the volume', async () => {
	const changed = vi.fn();
	device.onLevelChanged = changed;
	await connected();
	expect(prefs()).toEqual([{ player: MAC, cmd: ['playerpref', 'digitalVolumeControl', '0'] }]);
	// It read this player's own status first (a command the gateway allows).
	expect(lms.sent[0]).toEqual({ player: MAC, cmd: ['status', '-', 1, 'tags:al'] });
	expect(changed).toHaveBeenCalledTimes(1);
	expect(device.log.some((l) => l.includes('page: asked LMS to fix the output level at 100% (this player connected)'))).toBe(true);
});

test('sends nothing when LMS already agrees, or reports no Volume Control at all', async () => {
	lms.control = 0;
	await connected();
	expect(prefs()).toEqual([]);
	device.stop();
	lms.control = undefined;
	await connected();
	expect(prefs()).toEqual([]);
});

test('turned off: gives LMS the volume back (sends 1), and only to this player', async () => {
	lms.control = 0;
	await connected();
	expect(prefs()).toEqual([]);
	device.setFixedLevel(false);
	await settle();
	expect(prefs()).toEqual([{ player: MAC, cmd: ['playerpref', 'digitalVolumeControl', '1'] }]);
	expect(localStorage.getItem('anywhere.device.fixedLevel')).toBe('off');
	// Turned on again: fixed again, and the stored setting is the default once more.
	device.setFixedLevel(true);
	await settle();
	expect(prefs().map((p) => p.cmd[2])).toEqual(['1', '0']);
	expect(localStorage.getItem('anywhere.device.fixedLevel')).toBeNull();
});

test('turned off on connecting: sends 1 when LMS has the level fixed', async () => {
	device.setFixedLevel(false);
	lms.control = 0;
	await connected();
	expect(prefs()).toEqual([{ player: MAC, cmd: ['playerpref', 'digitalVolumeControl', '1'] }]);
});

test('changing the setting while not connected only saves it', async () => {
	device.setFixedLevel(false);
	await settle();
	expect(lms.sent).toEqual([]);
});

test('a failure is noted and not retried', async () => {
	lms.failPref = true;
	await connected();
	await new Promise((r) => setTimeout(r, 50));
	expect(prefs()).toHaveLength(1);
	expect(device.log.some((l) => l.includes('could not set this player’s Volume Control: refused'))).toBe(true);

	device.stop();
	lms.sent = [];
	lms.failPref = false;
	lms.failStatus = true;
	await connected();
	await new Promise((r) => setTimeout(r, 50));
	expect(lms.sent.filter((s) => s.cmd[0] === 'status')).toHaveLength(1);
	expect(prefs()).toEqual([]);
	expect(device.log.some((l) => l.includes('could not read this player’s Volume Control: LMS is busy'))).toBe(true);
});

test('a controller that turns it back is answered at most every 30 s', async () => {
	await connected();
	expect(prefs()).toHaveLength(1);
	// A controller gives LMS the volume back at once: inside 30 s of the last set, nothing.
	device.observeStatus(MAC, { digital_volume_control: 1 });
	await settle();
	expect(prefs()).toHaveLength(1);
	clock += FIXED_LEVEL_EVERY_MS;
	device.observeStatus(MAC, { digital_volume_control: 1 });
	device.observeStatus(MAC, { digital_volume_control: 1 });
	await settle();
	expect(prefs()).toHaveLength(2);
	expect(prefs()[1]).toEqual({ player: MAC, cmd: ['playerpref', 'digitalVolumeControl', '0'] });
	clock += FIXED_LEVEL_EVERY_MS - 1;
	device.observeStatus(MAC, { digital_volume_control: 1 });
	await settle();
	expect(prefs()).toHaveLength(2);
	clock += 1;
	device.observeStatus(MAC, { digital_volume_control: 1 });
	await settle();
	expect(prefs()).toHaveLength(3);
});

test('the status of another player, an agreeing one, or one without the field changes nothing', async () => {
	lms.control = 0;
	await connected();
	clock += 2 * FIXED_LEVEL_EVERY_MS;
	device.observeStatus('02:4c:41:00:00:02', { digital_volume_control: 1 });
	device.observeStatus(MAC, { digital_volume_control: 0 });
	device.observeStatus(MAC, {});
	device.observeStatus(MAC, null);
	device.observeStatus(MAC, { digital_volume_control: 'loud' as unknown as number });
	device.observeStatus(null, { digital_volume_control: 1 });
	await settle();
	expect(prefs()).toEqual([]);
});

test('not connected: a disagreeing status of this player sends nothing', async () => {
	device.mac = MAC;
	device.observeStatus(MAC, { digital_volume_control: 1 });
	await settle();
	expect(lms.sent).toEqual([]);
});
