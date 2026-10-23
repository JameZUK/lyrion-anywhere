// Player-first (SPEC D20): the output button opens "Play on…", which lists this device (ticked
// when in use) and the other players with what each is playing. Wave 3.5: tapping a row plays
// on that player; moving the playback there is in the row's ⋯; the mini player says which
// player is in use ("on Lounge"); the old "Playing on" strip is gone, and moving playback
// is said in the one status line.
import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';

vi.mock('#lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const fake = await import('./helpers/fake-app.svelte');
const { default: OutputButton } = await import('#lib/components/OutputButton.svelte');
const { default: PlayerSwitcher } = await import('#lib/components/PlayerSwitcher.svelte');
const { default: StatusSlot } = await import('#lib/components/StatusSlot.svelte');
const { default: PlayerBar } = await import('#lib/components/PlayerBar.svelte');
const { choose, openMenu } = await import('./helpers/menu');
const { device } = await import('#lib/player/device.svelte');

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
	expect(r.queryByRole('button', { name: /^Play on Firefox on Linux/ })).toBeNull();
	await waitFor(() => expect(r.container.textContent).toContain('Playing So What · Miles Davis'));
	expect(r.container.textContent).toContain('Off');
});

test('tapping a row plays on that player and closes; nothing is moved', async () => {
	const onclose = vi.fn();
	const r = render(PlayerSwitcher, { onclose });
	await fireEvent.click(r.getByRole('button', { name: 'Play on Lounge' }));
	expect(fake.app.playerId).toBe(LOUNGE);
	expect(onclose).toHaveBeenCalled();
	expect(fake.moves).toEqual([]);
});

test('a row’s ⋯ › Move playback to … moves playback from the player in use to that one', async () => {
	const r = render(PlayerSwitcher, { onclose: () => {} });
	// One trailing control per row: no Move pill, the action is in the menu.
	expect(r.queryByRole('button', { name: /^Move/ })).toBeNull();
	await choose('More options for Study', 'Move playback to Study', r.container);
	await waitFor(() => expect(fake.moves).toEqual([{ source: ME, target: STUDY, name: 'Study' }]));
});

test('nothing queued: nothing to move, so no Move in any row’s menu', async () => {
	fake.app.now = { mode: 'stop', playlist_tracks: 0 };
	const r = render(PlayerSwitcher, { onclose: () => {} });
	await waitFor(() => expect(r.container.textContent).toContain('Playing So What'));
	for (const b of r.queryAllByRole('button', { name: /^More options for/ })) {
		await fireEvent.click(b);
		expect(r.queryAllByRole('menuitem', { name: /^Move playback/ })).toHaveLength(0);
		await fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
	}
	await fireEvent.click(r.getByRole('button', { name: 'More options for this device' }));
	expect(r.queryAllByRole('menuitem', { name: /^Move playback/ })).toHaveLength(0);
	// And no explanatory paragraph either.
	expect(r.container.textContent).not.toMatch(/carries on there from the same point/);
});

test('controlling another player: this device’s ⋯ moves the playback here', async () => {
	fake.app.playerId = LOUNGE;
	const r = render(PlayerSwitcher, { onclose: () => {} });
	// Not to itself: the player in use has no Move in its menu.
	await waitFor(() => expect(r.container.textContent).toContain('Playing So What'));
	await fireEvent.click(r.getByRole('button', { name: 'More options for Lounge' }));
	expect(r.queryAllByRole('menuitem', { name: /^Move playback/ })).toHaveLength(0);
	await fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
	await choose('More options for this device', 'Move playback to this device', r.container);
	await waitFor(() => expect(fake.moves).toEqual([{ source: LOUNGE, target: ME, name: 'this device' }]));
});

test('choosing this device starts its player if it is not running', async () => {
	fake.app.playerId = LOUNGE;
	device.state = 'off';
	const r = render(PlayerSwitcher, { onclose: () => {} });
	await fireEvent.click(r.getByRole('button', { name: /^Play on this device/ }));
	expect(fake.app.playerId).toBe(ME);
	expect(start).toHaveBeenCalledTimes(1);
});

test("a browser that can't play: this device can't be chosen, and says why", async () => {
	vi.stubGlobal('AudioWorkletNode', undefined);
	vi.stubGlobal('MediaSource', undefined);
	vi.stubGlobal('ManagedMediaSource', undefined);
	fake.app.playerId = LOUNGE;
	const r = render(PlayerSwitcher, { onclose: () => {} });
	const here = r.getByRole('button', { name: /^Play on this device/ }) as HTMLButtonElement;
	expect(here.disabled).toBe(true);
	expect(here.textContent).toContain("can't play music itself");
	// Its ⋯ offers no move here (its player's alarms only, as LMS lists it).
	await fireEvent.click(r.getByRole('button', { name: 'More options for this device' }));
	expect(r.queryAllByRole('menuitem', { name: /^Move playback/ })).toHaveLength(0);
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

describe('which player is in use (Wave 3.5: no "Playing on" strip)', () => {
	test('this device in use: the mini player names no player, and no strip shows', () => {
		const r = render(PlayerBar);
		expect(r.container.querySelector('[data-mini-on]')).toBeNull();
		expect(r.container.querySelector('[data-strip]')).toBeNull();
	});

	test('another player: the mini player says "on Lounge", first on its line; still no strip', () => {
		fake.app.playerId = LOUNGE;
		fake.app.now = { mode: 'play', playlist_tracks: 6, playlist_loop: [{ title: 'So What', artist: 'Miles Davis', album: 'Kind of Blue' }] };
		const r = render(PlayerBar);
		const on = r.container.querySelector('[data-mini-on]')!;
		expect(on.textContent).toBe('on Lounge');
		expect(on.parentElement!.textContent!.trim()).toBe('on Lounge · Miles Davis · Kind of Blue');
		expect(r.getByRole('link', { name: /^Now playing: So What, on Lounge/ })).toBeTruthy();
		expect(r.container.querySelector('[data-strip]')).toBeNull();
		expect(r.queryByRole('button', { name: /Bring|Back to this device/ })).toBeNull();
	});

	test('a group is named by its members', () => {
		fake.app.playerId = LOUNGE;
		fake.app.now = { mode: 'play', playlist_tracks: 2, sync_master: LOUNGE, sync_slaves: STUDY };
		const r = render(PlayerBar);
		expect(r.container.querySelector('[data-mini-on]')!.textContent).toBe('on Lounge + Study');
	});

	test('no player chosen: the mini player says so, and how to choose one', () => {
		fake.app.playerId = '';
		fake.app.now = null;
		const r = render(PlayerBar);
		expect(r.container.querySelector('.title')!.textContent).toBe('No player chosen');
		expect(r.container.querySelector('.sub')!.textContent).toContain('Choose a player');
	});

	test('only while playback is moving does the status line say so, and where to', () => {
		fake.app.playerId = LOUNGE;
		let r = render(StatusSlot);
		expect(r.container.querySelector('[data-status]')).toBeNull();
		r.unmount();
		fake.app.moving = 'Study';
		r = render(StatusSlot);
		expect(r.container.querySelector('[data-status="moving"]')!.textContent).toContain('Moving playback to Study…');
		expect(r.queryAllByRole('button')).toHaveLength(0);
		fake.app.moving = '';
	});

	test('what the strip offered is in Play on…: back to this device, and moving the playback here', async () => {
		fake.app.playerId = LOUNGE;
		const r = render(PlayerSwitcher, { onclose: () => {} });
		const menu = await openMenu('More options for this device', r.container);
		expect(menu.textContent).toContain('Move playback to this device');
		await fireEvent.keyDown(menu, { key: 'Escape' });
		await fireEvent.click(r.getByRole('button', { name: /^Play on this device/ }));
		expect(fake.app.playerId).toBe(ME);
		expect(fake.moves).toEqual([]);
	});
});
