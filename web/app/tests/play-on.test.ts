// Player-first (SPEC D20): the small output button by the volume opens "Play on…", which
// lists this device (ticked when in use) and the other players with what each is playing;
// each other player can be controlled or have playback moved to it. While another player is
// in use, a strip says so and offers one tap back.
import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const fake = await import('./helpers/fake-app.svelte');
const { default: OutputButton } = await import('$lib/components/OutputButton.svelte');
const { default: PlayerSwitcher } = await import('$lib/components/PlayerSwitcher.svelte');
const { default: PlayingElsewhere } = await import('$lib/components/PlayingElsewhere.svelte');
const { device } = await import('$lib/player/device.svelte');

const ME = '02:4c:41:00:00:01';
const LOUNGE = 'aa:bb:cc:dd:ee:01';
const STUDY = 'aa:bb:cc:dd:ee:02';

class FakeWorklet {}
let start: ReturnType<typeof vi.spyOn>;
beforeEach(() => {
	vi.stubGlobal('AudioWorkletNode', FakeWorklet);
	vi.stubGlobal('Worker', class {});
	fake.calls.length = 0;
	fake.moves.length = 0;
	fake.app.deviceMac = ME;
	fake.app.players = [
		{ playerid: ME, name: 'Firefox on Linux' },
		{ playerid: LOUNGE, name: 'Lounge', isplaying: 1 },
		{ playerid: STUDY, name: 'Study', power: 0 },
	];
	fake.app.playerId = ME;
	fake.app.now = { mode: 'play', playlist_tracks: 6 };
	fake.handler.rpc = async (player, cmd) => {
		if (cmd[0] === 'status' && player === LOUNGE) return { mode: 'play', playlist_loop: [{ title: 'So What', artist: 'Miles Davis' }] };
		if (cmd[0] === 'status' && player === STUDY) return { mode: 'stop', power: 0 };
		return {};
	};
	device.state = 'connected';
	device.mac = ME;
	start = vi.spyOn(device, 'start').mockResolvedValue(undefined);
});
afterEach(() => {
	vi.unstubAllGlobals();
	start.mockRestore();
	device.state = 'off';
	device.mac = '';
});

test('the output button opens Play on… and toggles it shut', async () => {
	const r = render(OutputButton);
	const btn = r.getByRole('button', { name: 'Play on…' });
	expect(btn.getAttribute('aria-expanded')).toBe('false');
	await fireEvent.click(btn);
	expect(r.getByRole('dialog', { name: 'Play on' })).toBeTruthy();
	expect(btn.getAttribute('aria-expanded')).toBe('true');
	await fireEvent.click(btn);
	expect(r.queryByRole('dialog')).toBeNull();
});

test('while another player is in use, the output button says which', async () => {
	fake.app.playerId = LOUNGE;
	const r = render(OutputButton);
	expect(r.getByRole('button', { name: 'Play on… (now Lounge)' })).toBeTruthy();
});

test('this device first, ticked when in use; other players show what they play', async () => {
	const r = render(PlayerSwitcher, { onclose: () => {} });
	const rows = r.container.querySelectorAll('.output');
	expect(rows[0]!.textContent).toContain('This device');
	expect(rows[0]!.querySelector('[aria-current="true"]')).not.toBeNull();
	expect(r.getAllByRole('img', { name: 'In use' })).toHaveLength(1);
	// This device's own player is not listed again among the others.
	expect(r.queryByRole('button', { name: 'Control Firefox on Linux' })).toBeNull();
	await waitFor(() => expect(r.container.textContent).toContain('Playing So What · Miles Davis'));
	expect(r.container.textContent).toContain('Off');
});

test('Control switches to the player and closes; nothing is moved', async () => {
	const onclose = vi.fn();
	const r = render(PlayerSwitcher, { onclose });
	await fireEvent.click(r.getByRole('button', { name: 'Control Lounge' }));
	expect(fake.app.playerId).toBe(LOUNGE);
	expect(onclose).toHaveBeenCalled();
	expect(fake.moves).toEqual([]);
});

test('Move here moves playback from the player in use to that one', async () => {
	const r = render(PlayerSwitcher, { onclose: () => {} });
	await fireEvent.click(r.getByRole('button', { name: 'Move playback to Study' }));
	await waitFor(() => expect(fake.moves).toEqual([{ source: ME, target: STUDY, name: 'Study' }]));
});

test('nothing queued: nothing to move, so no Move here', async () => {
	fake.app.now = { mode: 'stop', playlist_tracks: 0 };
	const r = render(PlayerSwitcher, { onclose: () => {} });
	await tick();
	expect(r.queryAllByRole('button', { name: /^Move playback/ })).toHaveLength(0);
	expect(r.queryByRole('button', { name: 'Bring playback to this device' })).toBeNull();
});

test('controlling another player: this device offers to bring the playback here', async () => {
	fake.app.playerId = LOUNGE;
	const r = render(PlayerSwitcher, { onclose: () => {} });
	expect(r.queryByRole('button', { name: 'Move playback to Lounge' }), 'not to itself').toBeNull();
	await fireEvent.click(r.getByRole('button', { name: 'Bring playback to this device' }));
	await waitFor(() => expect(fake.moves).toEqual([{ source: LOUNGE, target: ME, name: 'this device' }]));
});

test('choosing this device starts its player if it is not running', async () => {
	fake.app.playerId = LOUNGE;
	device.state = 'off';
	const r = render(PlayerSwitcher, { onclose: () => {} });
	await fireEvent.click(r.getByRole('button', { name: /This device/ }));
	expect(fake.app.playerId).toBe(ME);
	expect(start).toHaveBeenCalledTimes(1);
});

test("a browser that can't play: this device can't be chosen, and says why", async () => {
	vi.stubGlobal('AudioWorkletNode', undefined);
	vi.stubGlobal('MediaSource', undefined);
	vi.stubGlobal('ManagedMediaSource', undefined);
	fake.app.playerId = LOUNGE;
	const r = render(PlayerSwitcher, { onclose: () => {} });
	const here = r.getByRole('button', { name: /This device/ }) as HTMLButtonElement;
	expect(here.disabled).toBe(true);
	expect(here.textContent).toContain("can't play music itself");
	expect(r.queryByRole('button', { name: 'Bring playback to this device' })).toBeNull();
});

test('hostile player names and titles stay text', async () => {
	fake.app.players = [...fake.app.players, { playerid: 'aa:bb:cc:dd:ee:03', name: '<img src=x onerror=alert(1)>' }];
	fake.handler.rpc = async (player, cmd) =>
		cmd[0] === 'status' ? { mode: 'play', playlist_loop: [{ title: '<script>alert(1)</script>' }] } : {};
	const r = render(PlayerSwitcher, { onclose: () => {} });
	await waitFor(() => expect(r.container.textContent).toContain('<script>alert(1)</script>'));
	expect(r.container.querySelector('img[src="x"], script')).toBeNull();
	expect(r.container.textContent).toContain('<img src=x onerror=alert(1)>');
});

describe('the "Playing on …" strip', () => {
	test('hidden while this device is in use', () => {
		const r = render(PlayingElsewhere);
		expect(r.container.querySelector('[data-strip]')).toBeNull();
	});

	test('names the other player, with one tap back to this device', async () => {
		fake.app.playerId = LOUNGE;
		const r = render(PlayingElsewhere);
		expect(r.getByRole('status').textContent).toContain('Playing on Lounge');
		await fireEvent.click(r.getByRole('button', { name: 'Back to this device' }));
		expect(fake.app.playerId).toBe(ME);
		expect(start, 'already running: not started again').not.toHaveBeenCalled();
		expect(fake.moves).toEqual([]);
	});

	test('Bring here moves the playback to this device', async () => {
		fake.app.playerId = LOUNGE;
		const r = render(PlayingElsewhere);
		await fireEvent.click(r.getByRole('button', { name: 'Bring playback to this device' }));
		await waitFor(() => expect(fake.moves).toEqual([{ source: LOUNGE, target: ME, name: 'this device' }]));
	});

	test('a group is named by its members', () => {
		fake.app.playerId = LOUNGE;
		fake.app.now = { mode: 'play', playlist_tracks: 2, sync_master: LOUNGE, sync_slaves: STUDY };
		const r = render(PlayingElsewhere);
		expect(r.getByRole('status').textContent).toContain('Playing on Lounge + Study');
	});

	test('while moving, it says where to', () => {
		fake.app.moving = 'Study';
		const r = render(PlayingElsewhere);
		expect(r.getByRole('status').textContent).toContain('Moving playback to Study…');
		fake.app.moving = '';
	});

	test("a browser that can't play: no offer to come back here", () => {
		vi.stubGlobal('AudioWorkletNode', undefined);
		vi.stubGlobal('MediaSource', undefined);
		vi.stubGlobal('ManagedMediaSource', undefined);
		fake.app.playerId = LOUNGE;
		const r = render(PlayingElsewhere);
		expect(r.getByRole('status').textContent).toContain('Playing on Lounge');
		expect(r.queryAllByRole('button')).toHaveLength(0);
	});
});
