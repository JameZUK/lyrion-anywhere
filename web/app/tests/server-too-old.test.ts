// A server too old for this browser's player (protocol 2, docs/PROTOCOL.md *Versions*): the
// status line says so on every page, and a manual start ("This device", "Move playback to
// this device") is refused with that text, never left waiting or silently switched elsewhere.
import { render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { OUTDATED_TEXT, RECONNECTING_TEXT, SERVER_TOO_OLD_TEXT, statusOf, type StatusInput } from '#lib/state/status';

vi.mock('#lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const fake = await import('./helpers/fake-app.svelte');
const { device } = await import('#lib/player/device.svelte');
const { bringHere, useThisDevice } = await import('#lib/player/outputs');
const { default: StatusSlot } = await import('#lib/components/StatusSlot.svelte');

const HERE = '02:4c:41:00:00:01';
const KITCHEN = '02:00:00:00:00:02';
const NONE: StatusInput = { moving: '', reach: 'online', message: '', reconnecting: false, heldBack: false, resume: false };

let starts = 0;
/** What a start learns from the gateway: too old, or fine. */
let startLearnsTooOld = false;

beforeEach(() => {
	fake.app.players = [{ playerid: KITCHEN, name: 'Kitchen' }];
	fake.app.deviceMac = HERE;
	fake.app.playerId = KITCHEN;
	fake.app.notice = '';
	fake.app.moving = '';
	fake.app.deviceFailures = 0;
	fake.moves.length = 0;
	device.state = 'off';
	device.mac = '';
	device.wasActive = false;
	device.outdated = false;
	device.serverTooOld = false;
	starts = 0;
	startLearnsTooOld = false;
	vi.spyOn(device, 'supported', 'get').mockReturnValue(true);
	vi.spyOn(device, 'start').mockImplementation(async () => {
		starts++;
		if (startLearnsTooOld) device.noteServerProtocol(1);
	});
});
afterEach(() => vi.restoreAllMocks());

describe('the status line', () => {
	test('says the server is too old, with the text Settings uses; after a stale page, before reconnecting', () => {
		expect(statusOf({ ...NONE, serverTooOld: true })).toEqual({ kind: 'server-too-old', text: SERVER_TOO_OLD_TEXT });
		expect(SERVER_TOO_OLD_TEXT).toBe("This browser can't play from this server until its Lyrion Anywhere plugin is updated to 0.7.3 or later. You can still control other players.");
		expect(statusOf({ ...NONE, serverTooOld: true, outdated: true })).toEqual({ kind: 'outdated', text: OUTDATED_TEXT });
		expect(statusOf({ ...NONE, serverTooOld: true, reconnecting: true })!.kind).toBe('server-too-old');
		expect(statusOf({ ...NONE, reconnecting: true })).toEqual({ kind: 'reconnecting', text: RECONNECTING_TEXT });
	});

	test('the slot shows it on every page while the device knows it', async () => {
		const r = render(StatusSlot);
		await tick();
		expect(r.container.querySelector('[data-status]')).toBeNull();
		device.serverTooOld = true;
		await tick();
		const slot = r.container.querySelector('[data-status=server-too-old]')!;
		expect(slot.textContent).toContain(SERVER_TOO_OLD_TEXT);
		expect(slot.getAttribute('role')).toBe('status');
		// Nothing to press: only a plugin update fixes it.
		expect(slot.querySelector('button')).toBeNull();
		r.unmount();
	});
});

describe('noteServerProtocol', () => {
	test.each([
		[1, true],
		[null, true],
		[2, false],
		[3, false],
		[undefined, false],
	] as const)('protocol %s: too old %s', (protocol, tooOld) => {
		expect(device.noteServerProtocol(protocol)).toBe(tooOld);
		expect(device.serverTooOld).toBe(tooOld);
	});

	test('noted in the connection log once', () => {
		const before = device.log.length;
		device.noteServerProtocol(1);
		device.noteServerProtocol(1);
		expect(device.log.slice(before).filter((l) => /update the server's plugin/.test(l))).toHaveLength(1);
	});
});

describe('This device, chosen by hand', () => {
	test('already known: refused with the text; not selected, not started, nothing to wait for', async () => {
		device.serverTooOld = true;
		await useThisDevice();
		expect(fake.app.notice).toBe(SERVER_TOO_OLD_TEXT);
		expect(fake.app.playerId).toBe(KITCHEN);
		expect(starts).toBe(0);
		expect(fake.app.deviceFailures).toBe(0);
	});

	test('learnt while starting: the wait for this device ends at once, and the text says why', async () => {
		startLearnsTooOld = true;
		await useThisDevice();
		expect(starts).toBe(1);
		expect(fake.app.deviceFailures).toBe(1);
		expect(fake.app.notice).toBe(SERVER_TOO_OLD_TEXT);
	});

	test('an up-to-date server: selected and started, no notice', async () => {
		await useThisDevice();
		expect(fake.app.playerId).toBe(HERE);
		expect(starts).toBe(1);
		expect(fake.app.notice).toBe('');
		expect(fake.app.deviceFailures).toBe(0);
	});
});

describe('Move playback to this device', () => {
	test('already known: refused with the text; playback stays, nothing started', async () => {
		device.serverTooOld = true;
		expect(await bringHere()).toBe(false);
		expect(fake.app.notice).toBe(SERVER_TOO_OLD_TEXT);
		expect(starts).toBe(0);
		expect(fake.moves).toEqual([]);
		expect(fake.app.playerId).toBe(KITCHEN);
	});

	test('learnt while starting: refused with the text, never "moving"', async () => {
		startLearnsTooOld = true;
		expect(await bringHere()).toBe(false);
		expect(fake.app.notice).toBe(SERVER_TOO_OLD_TEXT);
		expect(fake.app.moving).toBe('');
		expect(fake.moves).toEqual([]);
		expect(fake.app.playerId).toBe(KITCHEN);
	});
});
