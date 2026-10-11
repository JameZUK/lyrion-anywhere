// The player picker's sync section: it shows groups from LMS, lists only the players this
// user may control, and sends the right sync command for the right player.
import { fireEvent, render, waitFor } from '@testing-library/svelte';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const fake = await import('./helpers/fake-app.svelte');
const { default: PlayerSwitcher } = await import('$lib/components/PlayerSwitcher.svelte');

const A = '02:00:00:00:00:0a';
const B = '02:00:00:00:00:0b';
const C = '02:00:00:00:00:0c';
const HIDDEN = '02:00:00:00:00:ff';
let groups: string[] = [];
let refuse = false;

beforeEach(() => {
	fake.calls.length = 0;
	refuse = false;
	groups = [];
	fake.app.players = [
		{ playerid: A, name: 'Lounge' },
		{ playerid: B, name: 'Study' },
		{ playerid: C, name: 'Kitchen' },
	];
	fake.app.playerId = A;
	fake.app.now = null;
	fake.handler.rpc = async (_p, cmd) => {
		if (cmd[0] === 'syncgroups') return { syncgroups_loop: groups.map((m) => ({ sync_members: m })) };
		if (cmd[0] === 'sync' && refuse) throw new Error('refused');
		return {};
	};
});

const syncCalls = () => fake.calls.filter((c) => c.cmd[0] === 'sync');

test('lists the other permitted players as switches, and never a hidden one', async () => {
	groups = [`${A},${B},${HIDDEN}`];
	const r = render(PlayerSwitcher, { onclose: () => {} });
	await waitFor(() => expect(r.getByRole('switch', { name: 'Sync with Study' }).getAttribute('aria-checked')).toBe('true'));
	expect(r.getByRole('switch', { name: 'Sync with Kitchen' }).getAttribute('aria-checked')).toBe('false');
	expect(r.queryByRole('switch', { name: 'Sync with Lounge' }), 'not the selected player itself').toBeNull();
	expect(r.getAllByRole('switch')).toHaveLength(2);
	expect(r.container.textContent).not.toContain(HIDDEN);
	expect(r.container.textContent).toContain('with Study'); // the group shows in the player list
	expect(fake.calls[0]).toEqual({ player: '', cmd: ['syncgroups', '?'] });
});

test('playing alone: switching another player on makes this one join its group', async () => {
	const r = render(PlayerSwitcher, { onclose: () => {} });
	await waitFor(() => expect(fake.calls.length).toBeGreaterThan(0));
	await fireEvent.click(r.getByRole('switch', { name: 'Sync with Kitchen' }));
	await waitFor(() => expect(syncCalls()).toEqual([{ player: A, cmd: ['sync', C] }]));
	// The groups are read again afterwards.
	await waitFor(() => expect(fake.calls.filter((c) => c.cmd[0] === 'syncgroups').length).toBe(2));
});

// Regression: this used to send `<Kitchen> sync <Lounge>`, which in LMS moves Lounge out of
// its group and onto Kitchen's queue.
test('grouped: another player joins this group; switching a member off unsyncs that member', async () => {
	groups = [`${A},${B}`];
	const r = render(PlayerSwitcher, { onclose: () => {} });
	await waitFor(() => expect(r.getByRole('switch', { name: 'Sync with Study' }).getAttribute('aria-checked')).toBe('true'));
	await fireEvent.click(r.getByRole('switch', { name: 'Sync with Kitchen' }));
	await waitFor(() => expect(syncCalls()).toEqual([{ player: A, cmd: ['sync', C] }]));
	await waitFor(() => expect(r.getByRole('switch', { name: 'Sync with Study' })).not.toHaveProperty('disabled', true));
	await fireEvent.click(r.getByRole('switch', { name: 'Sync with Study' }));
	await waitFor(() => expect(syncCalls()[1]).toEqual({ player: B, cmd: ['sync', '-'] }));
});

test('"on its own" takes the selected player out of its group', async () => {
	groups = [`${A},${B}`];
	const r = render(PlayerSwitcher, { onclose: () => {} });
	const leave = await r.findByRole('button', { name: 'Play Lounge on its own' });
	await fireEvent.click(leave);
	await waitFor(() => expect(syncCalls()).toEqual([{ player: A, cmd: ['sync', '-'] }]));
});

test('the group is known from the player status too', async () => {
	fake.app.now = { sync_master: B, sync_slaves: A };
	const r = render(PlayerSwitcher, { onclose: () => {} });
	await waitFor(() => expect(r.getByRole('switch', { name: 'Sync with Study' }).getAttribute('aria-checked')).toBe('true'));
});

test('a refused sync is reported', async () => {
	refuse = true;
	const r = render(PlayerSwitcher, { onclose: () => {} });
	await waitFor(() => expect(fake.calls.length).toBeGreaterThan(0));
	await fireEvent.click(r.getByRole('switch', { name: 'Sync with Study' }));
	expect((await r.findByRole('alert')).textContent).toContain("Couldn't change the group");
});

test('no sync section with only one player, and no group query sent', async () => {
	fake.app.players = [{ playerid: A, name: 'Lounge' }];
	const r = render(PlayerSwitcher, { onclose: () => {} });
	await new Promise((res) => setTimeout(res, 10));
	expect(r.queryAllByRole('switch')).toHaveLength(0);
	expect(fake.calls.filter((c) => c.cmd[0] !== 'status')).toEqual([]);
});

test('hostile player names stay text', async () => {
	fake.app.players = [
		{ playerid: A, name: 'Lounge' },
		{ playerid: B, name: '<img src=x onerror=alert(1)>' },
	];
	const r = render(PlayerSwitcher, { onclose: () => {} });
	await r.findByRole('switch', { name: 'Sync with <img src=x onerror=alert(1)>' });
	expect(r.container.querySelectorAll('img').length).toBe(0);
});
