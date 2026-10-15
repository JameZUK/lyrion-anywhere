// Rooms (Wave 2 §4.3; Wave 3.5 item 13): the poller (what it reads, how often, for whom, and
// that it stops), and the page: one labelled group per room with three controls (play/pause,
// volume, ⋯) named after the room, sync by menu or by dragging the cover, unsync, Transfer,
// power in ⋯, and nothing read once it closes. Play on… shares the same player manager.
import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import type { AlarmListing } from '$lib/api/routine';
import type { PlayerInfo, PlayerStatus } from '$lib/lms/types';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const alarmCalls: string[] = [];
let alarmAnswer: (p: string) => Promise<AlarmListing> = async () => ({ enabled: true, alarms: [], sounds: [] });
vi.mock('$lib/state/me', () => ({
	routine: {
		alarms: (p: string) => {
			alarmCalls.push(p);
			return alarmAnswer(p);
		},
	},
}));
const fake = await import('./helpers/fake-app.svelte');
const { MAX_ROOMS, POLL_MS, EVENT_DELAY_MS, RoomsPoller, alarmText, roomStatus } = await import('$lib/rooms/rooms.svelte');
const { alarmChanges } = await import('$lib/rooms/alarm-changes.svelte');
const { goto } = await import('$app/navigation');
const { default: Rooms } = await import('../src/routes/rooms/+page.svelte');
const { default: PlayerSwitcher } = await import('$lib/components/PlayerSwitcher.svelte');

const KITCHEN = '02:00:00:00:00:01';
const LOUNGE = '02:00:00:00:00:02';
const STUDY = '02:00:00:00:00:03';

function players(n = 3): PlayerInfo[] {
	const named: PlayerInfo[] = [
		{ playerid: KITCHEN, name: 'Kitchen' },
		{ playerid: LOUNGE, name: 'Lounge' },
		{ playerid: STUDY, name: 'Study' },
	];
	for (let i = named.length; i < n; i++) named.push({ playerid: `02:00:00:00:01:${i.toString(16).padStart(2, '0')}`, name: `Room ${i}` });
	return named.slice(0, n);
}

const statuses: Record<string, PlayerStatus> = {
	[KITCHEN]: { mode: 'play', power: 1, 'mixer volume': 40, will_sleep_in: 600, playlist_loop: [{ id: 1, title: 'Morning tune', artist: 'A Band' }] },
	[LOUNGE]: { mode: 'pause', power: 1, 'mixer volume': 20, playlist_loop: [{ id: 2, title: 'Evening tune', artist: 'Another Band' }] },
	[STUDY]: { mode: 'stop', power: 0, 'mixer volume': 10 },
};
let groups: { sync_members: string }[] = [];

function rpcAnswer(p: string, c: (string | number)[]) {
	if (c[0] === 'status') return statuses[p] ?? { mode: 'stop', power: 1 };
	if (c[0] === 'syncgroups') return { syncgroups_loop: groups };
	return {};
}

beforeEach(() => {
	fake.calls.length = 0;
	alarmCalls.length = 0;
	groups = [];
	alarmAnswer = async () => ({ enabled: true, alarms: [], sounds: [] });
	fake.app.players = players();
	fake.app.playerId = LOUNGE;
	fake.app.deviceMac = '';
	fake.app.now = { playlist_tracks: 5, mode: 'pause' };
	fake.app.watchers.clear();
	fake.handler.rpc = async (p, c) => rpcAnswer(p, c);
	vi.mocked(goto).mockClear();
});
afterEach(() => vi.useRealTimers());

const sent = (name: string) => fake.calls.filter((c) => c.cmd[0] === name);

describe('the poller', () => {
	function poller(visible = () => true) {
		return new RoomsPoller({
			rpc: (p, c) => fake.api.rpc(p, c),
			alarms: async (p) => {
				alarmCalls.push(p);
				return alarmAnswer(p);
			},
			visible,
			now: () => new Date(2026, 9, 13, 6, 0, 0), // a Tuesday
		});
	}

	test('reads each room (and the groups) every 5 s while running, and nothing once stopped', async () => {
		vi.useFakeTimers();
		const p = poller();
		p.start(() => fake.app.players);
		await vi.advanceTimersByTimeAsync(0);
		expect(sent('status').map((c) => c.player).sort()).toEqual([KITCHEN, LOUNGE, STUDY]);
		// The exact command the gateway allows: `status - 1 tags:…`.
		expect(sent('status')[0]!.cmd).toEqual(roomStatus());
		expect(roomStatus()).toEqual(['status', '-', 1, 'tags:alcJ']);
		expect(sent('syncgroups')).toEqual([{ player: '', cmd: ['syncgroups', '?'] }]);
		expect(p.status[KITCHEN]?.mode).toBe('play');
		await vi.advanceTimersByTimeAsync(POLL_MS);
		expect(sent('status')).toHaveLength(6);
		p.stop();
		await vi.advanceTimersByTimeAsync(POLL_MS * 4);
		expect(sent('status')).toHaveLength(6);
		expect(p.running).toBe(false);
	});

	test('at most 20 players', async () => {
		fake.app.players = players(30);
		const p = poller();
		p.start(() => fake.app.players);
		await vi.waitFor(() => expect(sent('status')).toHaveLength(MAX_ROOMS));
		expect(p.rooms()).toHaveLength(20);
		p.stop();
	});

	test('skips rounds while the page is hidden', async () => {
		vi.useFakeTimers();
		let visible = true;
		const p = poller(() => visible);
		p.start(() => fake.app.players);
		await vi.advanceTimersByTimeAsync(0);
		visible = false;
		await vi.advanceTimersByTimeAsync(POLL_MS * 3);
		expect(sent('status')).toHaveLength(3);
		visible = true;
		await vi.advanceTimersByTimeAsync(POLL_MS);
		expect(sent('status')).toHaveLength(6);
		p.stop();
	});

	test('an event reads that room soon; others, unknown players and events after stopping are ignored', async () => {
		vi.useFakeTimers();
		const p = poller();
		p.start(() => fake.app.players);
		await vi.advanceTimersByTimeAsync(0);
		fake.calls.length = 0;
		p.onEvent({ player: KITCHEN.toUpperCase(), event: ['mixer', 'volume', '50'] });
		p.onEvent({ player: KITCHEN, event: ['mixer', 'volume', '55'] });
		p.onEvent({ player: '02:00:00:00:09:09', event: ['play'] });
		p.onEvent({ player: null, event: ['rescan'] });
		await vi.advanceTimersByTimeAsync(EVENT_DELAY_MS);
		expect(fake.calls.map((c) => c.player)).toEqual([KITCHEN]);
		// A sync event reads the groups too; a resync reads everything.
		fake.calls.length = 0;
		p.onEvent({ player: LOUNGE, event: ['sync'] });
		await vi.advanceTimersByTimeAsync(EVENT_DELAY_MS);
		expect(fake.calls.map((c) => c.cmd[0]).sort()).toEqual(['status', 'syncgroups']);
		fake.calls.length = 0;
		p.onEvent('resync');
		await vi.advanceTimersByTimeAsync(0);
		expect(sent('status')).toHaveLength(3);
		p.stop();
		fake.calls.length = 0;
		p.onEvent({ player: KITCHEN, event: ['play'] });
		p.onEvent('resync');
		await vi.advanceTimersByTimeAsync(POLL_MS);
		expect(fake.calls).toHaveLength(0);
	});

	test('answers that arrive after stopping are dropped', async () => {
		let release: () => void = () => {};
		fake.handler.rpc = (pl, c) => new Promise((r) => (release = () => r(rpcAnswer(pl, c))));
		const p = poller();
		p.start(() => [fake.app.players[0]!]);
		p.stop();
		release();
		await tick();
		expect(p.status).toEqual({});
	});

	test('the next alarm is read once on opening; off or failing gives none', async () => {
		alarmAnswer = async (pl) => {
			if (pl === STUDY) throw new Error('offline');
			return {
				enabled: pl === KITCHEN,
				alarms: [{ id: 'a1', time: 7 * 3600, days: [2], enabled: true, repeat: true, volume: null, shuffle: 0, sound: null }],
				sounds: [],
			};
		};
		const p = poller();
		p.start(() => fake.app.players);
		await vi.waitFor(() => expect(Object.keys(p.alarms)).toHaveLength(3));
		expect(p.alarms[KITCHEN]?.getHours()).toBe(7);
		expect(p.alarms[LOUNGE]).toBeNull();
		expect(p.alarms[STUDY]).toBeNull();
		expect(alarmCalls).toHaveLength(3);
		p.stop();
	});

	test('malformed player ids are never shown, read or sent to', async () => {
		fake.app.players = [...players(2), { playerid: '../../admin', name: 'Bad' }, { playerid: 'ff:ff:ff:ff:ff', name: 'Short' }];
		const p = poller();
		p.start(() => fake.app.players);
		await vi.waitFor(() => expect(sent('status')).toHaveLength(2));
		expect(p.rooms().map((r) => r.name)).toEqual(['Kitchen', 'Lounge']);
		await tick();
		expect(fake.calls.every((c) => c.player === '' || /^[0-9a-f]{2}(:[0-9a-f]{2}){5}$/.test(c.player))).toBe(true);
		p.stop();
	});

	test('alarm times read naturally', () => {
		const now = new Date(2026, 9, 13, 6, 0);
		expect(alarmText(new Date(2026, 9, 13, 7, 5), now)).toBe('today 07:05');
		expect(alarmText(new Date(2026, 9, 14, 7, 0), now)).toBe('tomorrow 07:00');
		expect(alarmText(new Date(2026, 9, 16, 7, 0), now)).toBe('Fri 07:00');
		expect(alarmText(null, now)).toBe('');
		expect(alarmText(new Date(NaN), now)).toBe('');
	});
});

describe('the Rooms page', () => {
	async function open() {
		const r = render(Rooms);
		await waitFor(() => expect(r.container.querySelector('[data-room-meta]')?.textContent).toContain('Playing'));
		return r;
	}
	const card = (r: { container: HTMLElement }, name: string) => r.container.querySelector<HTMLElement>(`[role="group"][aria-label="${name}"]`)!;

	test('one labelled group per room, three controls named after the room; no instructions paragraph', async () => {
		const r = await open();
		expect([...r.container.querySelectorAll('[data-room]')].map((c) => c.getAttribute('aria-label'))).toEqual(['Kitchen', 'Lounge', 'Study']);
		const k = card(r, 'Kitchen');
		// Play/pause, volume and ⋯: no power button, no grip (Wave 3.5).
		const controls = k.querySelector('.controls')!;
		expect([...controls.querySelectorAll('button, input')].map((e) => e.getAttribute('aria-label'))).toEqual(['Pause Kitchen', 'Kitchen volume', 'Kitchen options']);
		expect(k.querySelector('[data-room-grip]')).toBeNull();
		expect(card(r, 'Lounge').querySelector('[data-room-play]')!.getAttribute('aria-label')).toBe('Play Lounge');
		expect(k.querySelector('[data-room-control]')!.getAttribute('aria-label')).toBe('Play on Kitchen: playing, Morning tune by A Band');
		expect(k.textContent).toContain('sleep in 10 min');
		// A switched-off room has one Switch on button in place of play/pause and volume.
		const st = card(r, 'Study');
		expect([...st.querySelector('.controls')!.querySelectorAll('button, input')].map((e) => e.textContent?.trim() || e.getAttribute('aria-label'))).toEqual(['Switch on', 'Study options']);
		// The room in use is marked.
		expect(card(r, 'Lounge').querySelector('[data-room-control]')!.getAttribute('aria-current')).toBe('true');
		expect(card(r, 'Lounge').querySelector('[data-room-control]')!.getAttribute('aria-label')).toMatch(/^Play on Lounge \(in use\)/);
		expect(r.container.textContent).not.toMatch(/Drag a room onto another/);
		r.unmount();
	});

	test('play/pause, volume and power go to that room only', async () => {
		const r = await open();
		fake.calls.length = 0;
		await fireEvent.click(card(r, 'Kitchen').querySelector('[data-room-play]')!);
		await fireEvent.click(card(r, 'Lounge').querySelector('[data-room-play]')!);
		// Power is in ⋯ now.
		await fireEvent.click(card(r, 'Kitchen').querySelector('[data-row-more]')!);
		await fireEvent.click([...card(r, 'Kitchen').querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((b) => b.textContent?.includes('Switch Kitchen off'))!);
		const vol = card(r, 'Lounge').querySelector<HTMLInputElement>('[data-room-volume]')!;
		vol.value = '64';
		await fireEvent.input(vol);
		await waitFor(() => expect(sent('mixer')).toHaveLength(1));
		const actions = fake.calls.filter((c) => c.cmd[0] !== 'status' && c.cmd[0] !== 'syncgroups');
		expect(actions).toEqual([
			{ player: KITCHEN, cmd: ['pause', 1] },
			{ player: LOUNGE, cmd: ['pause', 0] },
			{ player: KITCHEN, cmd: ['power', 0] },
			{ player: LOUNGE, cmd: ['mixer', 'volume', 64] },
		]);
		r.unmount();
	});

	test('a switched-off room switches on with its one button', async () => {
		const r = await open();
		fake.calls.length = 0;
		await fireEvent.click(card(r, 'Study').querySelector('[data-room-power]')!);
		await waitFor(() => expect(sent('power')).toEqual([{ player: STUDY, cmd: ['power', 1] }]));
		r.unmount();
	});

	test('tapping a card plays on that player', async () => {
		const r = await open();
		await fireEvent.click(card(r, 'Kitchen').querySelector('[data-room-control]')!);
		expect(fake.app.playerId).toBe(KITCHEN);
		expect(goto).toHaveBeenCalledWith('#/', {});
		r.unmount();
	});

	test('Sync with… from the card menu: the room joins the chosen one', async () => {
		const r = await open();
		await fireEvent.click(card(r, 'Kitchen').querySelector('[data-row-more]')!);
		const item = [...card(r, 'Kitchen').querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((b) => b.textContent?.includes('Sync with'))!;
		await fireEvent.click(item);
		const choices = await waitFor(() => document.querySelector('[data-sync-choices]')!);
		const labels = [...choices.querySelectorAll('button')].map((b) => b.getAttribute('aria-label'));
		expect(labels).toEqual(['Sync Kitchen with Lounge', 'Sync Kitchen with Study']);
		fake.calls.length = 0;
		await fireEvent.click(choices.querySelector('button')!);
		// `<Lounge> sync <Kitchen>`: Kitchen joins Lounge's group (never a player syncing itself).
		await waitFor(() => expect(sent('sync')).toEqual([{ player: LOUNGE, cmd: ['sync', KITCHEN] }]));
		expect(r.container.querySelector('[data-rooms-status]')!.textContent).toBe('Kitchen now plays in sync with Lounge.');
		r.unmount();
	});

	test('dragging a card by its cover onto another with a pointer syncs them', async () => {
		const r = await open();
		const target = card(r, 'Kitchen');
		const restore = document.elementFromPoint;
		document.elementFromPoint = ((x: number) => (x > 500 ? target : null)) as typeof document.elementFromPoint;
		try {
			const grip = card(r, 'Study').querySelector('[data-room-drag]')!;
			await fireEvent.pointerDown(grip, { pointerId: 3, clientX: 10, clientY: 10, button: 0 });
			await fireEvent.pointerMove(window, { pointerId: 3, clientX: 600, clientY: 10 });
			expect(target.classList.contains('target')).toBe(true);
			await fireEvent.pointerUp(window, { pointerId: 3, clientX: 600, clientY: 10 });
			await waitFor(() => expect(sent('sync')).toEqual([{ player: KITCHEN, cmd: ['sync', STUDY] }]));
			// Dropped on nothing: nothing sent.
			fake.calls.length = 0;
			await fireEvent.pointerDown(grip, { pointerId: 4, clientX: 10, clientY: 10, button: 0 });
			await fireEvent.pointerUp(window, { pointerId: 4, clientX: 10, clientY: 10 });
			await tick();
			expect(sent('sync')).toHaveLength(0);
		} finally {
			document.elementFromPoint = restore;
		}
		r.unmount();
	});

	test('a grouped room can play on its own again; Transfer moves playback to a room', async () => {
		groups = [{ sync_members: `${KITCHEN},${LOUNGE}` }];
		const r = await open();
		await waitFor(() => expect(card(r, 'Kitchen').textContent).toContain('with Lounge'));
		await fireEvent.click(card(r, 'Kitchen').querySelector('[data-row-more]')!);
		const items = () => [...card(r, 'Kitchen').querySelectorAll<HTMLButtonElement>('[role="menuitem"]')];
		expect(items().map((b) => b.textContent?.trim())).toEqual(['Play on Kitchen', 'Sync with…', 'Move playback to Kitchen', 'Pause Kitchen', 'Play on its own', 'Alarms', 'Switch Kitchen off']);
		fake.calls.length = 0;
		await fireEvent.click(items().find((b) => b.textContent?.includes('on its own'))!);
		await waitFor(() => expect(sent('sync')).toEqual([{ player: KITCHEN, cmd: ['sync', '-'] }]));
		await fireEvent.click(card(r, 'Study').querySelector('[data-row-more]')!);
		const move = [...card(r, 'Study').querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((b) => b.textContent?.includes('Move playback to Study'))!;
		fake.moves.length = 0;
		await fireEvent.click(move);
		expect(fake.moves).toEqual([{ source: LOUNGE, target: STUDY, name: 'Study' }]);
		r.unmount();
	});

	test('the next alarm shows on the card, and is read again when alarms change', async () => {
		alarmAnswer = async (pl) => ({
			enabled: true,
			alarms: pl === KITCHEN ? [{ id: 'a1', time: 6 * 3600 + 30 * 60, days: [0, 1, 2, 3, 4, 5, 6], enabled: true, repeat: true, volume: null, shuffle: 0, sound: null }] : [],
			sounds: [],
		});
		const r = await open();
		await waitFor(() => expect(card(r, 'Kitchen').textContent).toMatch(/alarm (today|tomorrow) 06:30/));
		expect(alarmCalls.sort()).toEqual([KITCHEN, LOUNGE, STUDY]);
		alarmChanges.bump();
		await waitFor(() => expect(alarmCalls).toHaveLength(6));
		r.unmount();
	});

	test('nothing is read once the page closes', async () => {
		vi.useFakeTimers();
		const r = render(Rooms);
		await vi.advanceTimersByTimeAsync(0);
		expect(sent('status')).toHaveLength(3);
		await vi.advanceTimersByTimeAsync(POLL_MS);
		expect(sent('status')).toHaveLength(6);
		expect(fake.app.watchers.size).toBe(1);
		r.unmount();
		expect(fake.app.watchers.size).toBe(0);
		fake.calls.length = 0;
		await vi.advanceTimersByTimeAsync(POLL_MS * 5);
		expect(fake.calls).toHaveLength(0);
	});

	test('events from the gateway refresh the room they are about', async () => {
		const r = await open();
		fake.calls.length = 0;
		statuses[LOUNGE] = { ...statuses[LOUNGE], 'mixer volume': 77 };
		fake.app.emit({ player: LOUNGE, event: ['mixer', 'volume', '77'] });
		await waitFor(() => expect(card(r, 'Lounge').querySelector<HTMLInputElement>('[data-room-volume]')!.value).toBe('77'));
		expect(sent('status').map((c) => c.player)).toEqual([LOUNGE]);
		r.unmount();
	});

	test('a room is never synced with itself, and volume stays within 0–100', async () => {
		const r = await open();
		// Dropping a card on itself does nothing.
		const k = card(r, 'Kitchen');
		const restore = document.elementFromPoint;
		document.elementFromPoint = (() => k) as typeof document.elementFromPoint;
		try {
			await fireEvent.pointerDown(k.querySelector('[data-room-drag]')!, { pointerId: 5, clientX: 1, clientY: 1, button: 0 });
			await fireEvent.pointerUp(window, { pointerId: 5, clientX: 2, clientY: 2 });
		} finally {
			document.elementFromPoint = restore;
		}
		await tick();
		expect(sent('sync')).toHaveLength(0);
		const vol = k.querySelector<HTMLInputElement>('[data-room-volume]')!;
		expect([vol.min, vol.max]).toEqual(['0', '100']);
		r.unmount();
	});

	test('Alarms in a card’s ⋯ opens that room’s alarms', async () => {
		const r = await open();
		await fireEvent.click(card(r, 'Kitchen').querySelector('[data-row-more]')!);
		await fireEvent.click([...card(r, 'Kitchen').querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((b) => b.textContent?.trim() === 'Alarms')!);
		expect(fake.app.playerId).toBe(KITCHEN);
		expect(goto).toHaveBeenCalledWith('#/alarms', {});
		r.unmount();
	});

	test('Play on… is the same manager: a row’s ⋯ holds what a card had (move, play/pause, alarms, power), and Join its group', async () => {
		const r = render(PlayerSwitcher, { onclose: () => {} });
		await waitFor(() => expect(r.container.textContent).toContain('Playing Morning tune'));
		await fireEvent.click(r.getByRole('button', { name: 'More options for Kitchen' }));
		const items = [...r.container.querySelectorAll<HTMLElement>('[role="menuitem"]')].map((b) => b.textContent?.trim());
		expect(items).toEqual(['Move playback to Kitchen', 'Pause Kitchen', 'Join its group', 'Alarms', 'Switch Kitchen off']);
		fake.calls.length = 0;
		await fireEvent.click([...r.container.querySelectorAll<HTMLElement>('[role="menuitem"]')].find((b) => b.textContent?.includes('Pause Kitchen'))!);
		await waitFor(() => expect(sent('pause')).toEqual([{ player: KITCHEN, cmd: ['pause', 1] }]));
		r.unmount();
	});

	test('Play on… reads statuses only while it is open', async () => {
		vi.useFakeTimers();
		const r = render(PlayerSwitcher, { onclose: () => {} });
		await vi.advanceTimersByTimeAsync(0);
		expect(sent('status').length).toBeGreaterThan(0);
		r.unmount();
		fake.calls.length = 0;
		await vi.advanceTimersByTimeAsync(POLL_MS * 3);
		expect(sent('status')).toHaveLength(0);
	});

	test('the player picker links to Rooms', async () => {
		const r = render(PlayerSwitcher, { onclose: () => {} });
		await tick();
		const link = r.container.querySelector<HTMLAnchorElement>('[data-rooms-link]')!;
		expect(link.getAttribute('href')).toBe('#/rooms');
		expect(link.textContent).toContain('Rooms');
		expect(link.textContent).not.toContain('All rooms'); // naming: Rooms is the page
		r.unmount();
	});
});
