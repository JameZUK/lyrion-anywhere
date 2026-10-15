// Play on…'s "Sync with…" row (2026-10-15, as on Android): under the controlled player's row,
// it opens a picker of the other players (groups collapsed to one entry, its own group marked)
// and the controlled player joins the chosen group. Each other row's ⋯ has "Join its group"
// (or "Leave the group" when that player is already in ours). The old "Group players…" row,
// which brings other players into this one's group, says so.
import { fireEvent, render, waitFor, within } from '@testing-library/svelte';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const fake = await import('./helpers/fake-app.svelte');
const { default: PlayerSwitcher } = await import('$lib/components/PlayerSwitcher.svelte');
const { choose, menuItems, openMenu } = await import('./helpers/menu');

const A = '02:00:00:00:00:0a';
const B = '02:00:00:00:00:0b';
const C = '02:00:00:00:00:0c';
const D = '02:00:00:00:00:0d';
const HIDDEN = '02:00:00:00:00:ff';
let groups: string[] = [];
let syncReply: () => Promise<unknown> = async () => ({});

beforeEach(() => {
	fake.calls.length = 0;
	groups = [];
	syncReply = async () => ({});
	fake.app.reach = 'online';
	fake.app.deviceMac = '';
	fake.app.players = [
		{ playerid: A, name: 'Lounge' },
		{ playerid: B, name: 'Study' },
		{ playerid: C, name: 'Kitchen' },
		{ playerid: D, name: 'Hall' },
	];
	fake.app.playerId = A;
	fake.app.now = null;
	fake.handler.rpc = async (_p, cmd) => {
		if (cmd[0] === 'syncgroups') return { syncgroups_loop: groups.map((m) => ({ sync_members: m })) };
		if (cmd[0] === 'sync') return syncReply();
		return {};
	};
});

const syncCalls = () => fake.calls.filter((c) => c.cmd[0] === 'sync');
const groupsRead = () => fake.calls.filter((c) => c.cmd[0] === 'syncgroups').length;

/** Play on…, once the sync groups have been read. */
async function open() {
	const r = render(PlayerSwitcher, { onclose: () => {} });
	await waitFor(() => expect(groupsRead()).toBeGreaterThan(0));
	return r;
}
type R = Awaited<ReturnType<typeof open>>;
const syncRow = (r: R) => r.container.querySelector<HTMLButtonElement>('[data-sync-row]');

/** The picker, opened from the row. */
async function picker(r: R) {
	await fireEvent.click(syncRow(r)!);
	return r.getByRole('dialog', { name: 'Sync with' });
}
const choices = (dlg: HTMLElement) => [...dlg.querySelectorAll<HTMLButtonElement>('[data-sync-choice]')];
const choiceName = (b: HTMLElement) => b.querySelector('.name')?.textContent?.trim();

test('playing alone: "Sync with…" sits directly under the controlled player\'s row, indented', async () => {
	const r = await open();
	const row = syncRow(r);
	expect(row, 'the row is there').toBeTruthy();
	expect(row!.textContent).toContain('Sync with…');
	const li = row!.closest('li')!;
	expect(li.classList.contains('sync-under')).toBe(true);
	expect(li.previousElementSibling?.querySelector(`[data-output="${A}"]`), 'right after the Lounge row').toBeTruthy();
});

test('grouped: the row says who it plays in sync with', async () => {
	groups = [`${A},${B},${HIDDEN}`];
	const r = await open();
	await waitFor(() => expect(syncRow(r)?.textContent).toContain('In sync with Study'));
	expect(r.container.textContent).not.toContain(HIDDEN);
});

test('the picker: every other player, groups collapsed to one entry', async () => {
	groups = [`${B},${C}`];
	const r = await open();
	const dlg = await picker(r);
	expect(within(dlg).getByRole('heading', { name: 'Sync Lounge with…' })).toBeTruthy();
	await waitFor(() => expect(choices(dlg).map(choiceName)).toEqual(['Study + Kitchen', 'Hall']));
	expect(within(dlg).queryByRole('button', { name: 'Leave the group' }), 'not grouped: nothing to leave').toBeNull();
});

test('grouped: its own group is one marked entry, and Leave the group is offered', async () => {
	groups = [`${A},${B}`];
	const r = await open();
	await waitFor(() => expect(syncRow(r)?.textContent).toContain('In sync with Study'));
	const dlg = await picker(r);
	const all = choices(dlg);
	expect(all.map(choiceName)).toEqual(['Lounge + Study', 'Kitchen', 'Hall']);
	const own = all[0]!;
	expect(own.getAttribute('aria-current')).toBe('true');
	expect(own.textContent).toContain('This group');
	expect(own.disabled, 'already in it').toBe(true);
	expect(within(dlg).getByRole('button', { name: 'Leave the group' })).toBeTruthy();
});

test('choosing a lone player: the controlled player joins it (sent for the chosen player)', async () => {
	const r = await open();
	const dlg = await picker(r);
	await fireEvent.click(choices(dlg).find((b) => choiceName(b) === 'Kitchen')!);
	// `<X> sync <Y>` makes Y join X's group (checked against LMS 9.1; see lib/lms/sync.ts).
	await waitFor(() => expect(syncCalls()).toEqual([{ player: C, cmd: ['sync', A] }]));
	// Back to the list, and the groups are read again.
	await waitFor(() => expect(r.getByRole('dialog', { name: 'Play on' })).toBeTruthy());
	await waitFor(() => expect(groupsRead()).toBe(2));
});

test('choosing a group: the controlled player joins that group', async () => {
	groups = [`${B},${C}`];
	const r = await open();
	const dlg = await picker(r);
	await waitFor(() => expect(choices(dlg)).toHaveLength(2));
	await fireEvent.click(choices(dlg)[0]!);
	await waitFor(() => expect(syncCalls()).toEqual([{ player: B, cmd: ['sync', A] }]));
});

test('Leave the group takes the controlled player out', async () => {
	groups = [`${A},${B}`];
	const r = await open();
	await waitFor(() => expect(syncRow(r)?.textContent).toContain('In sync'));
	const dlg = await picker(r);
	await fireEvent.click(within(dlg).getByRole('button', { name: 'Leave the group' }));
	await waitFor(() => expect(syncCalls()).toEqual([{ player: A, cmd: ['sync', '-'] }]));
});

test('the picker is keyboard operable: Back returns to the list', async () => {
	const r = await open();
	await picker(r);
	await fireEvent.click(r.getByRole('button', { name: 'Back to Play on' }));
	expect(r.getByRole('dialog', { name: 'Play on' })).toBeTruthy();
});

test("each other player's ⋯: Join its group", async () => {
	const r = await open();
	await choose('More options for Kitchen', 'Join its group', r.container);
	await waitFor(() => expect(syncCalls()).toEqual([{ player: C, cmd: ['sync', A] }]));
});

test("a player already in our group: its ⋯ says Leave the group, which takes this player out", async () => {
	groups = [`${A},${B}`];
	const r = await open();
	await waitFor(() => expect(syncRow(r)?.textContent).toContain('In sync'));
	const items = await menuItems('More options for Study', r.container);
	expect(items).toContain('Leave the group');
	expect(items).not.toContain('Join its group');
	await fireEvent.click(r.getByRole('button', { name: 'More options for Study' })); // close
	await choose('More options for Study', 'Leave the group', r.container);
	await waitFor(() => expect(syncCalls()).toEqual([{ player: A, cmd: ['sync', '-'] }]));
});

test('the controlled player itself has no Join its group', async () => {
	const r = await open();
	const items = await menuItems('More options for Lounge', r.container).catch(() => [] as string[]);
	expect(items).not.toContain('Join its group');
});

test('out of reach: the row and Join its group are disabled, saying why, and nothing is sent', async () => {
	fake.app.reach = 'unreachable';
	const r = render(PlayerSwitcher, { onclose: () => {} });
	await waitFor(() => expect(syncRow(r)).toBeTruthy());
	const row = syncRow(r)!;
	expect(row.disabled).toBe(true);
	expect(row.getAttribute('aria-describedby') || row.getAttribute('title')).toBeTruthy();
	const menu = await openMenu('More options for Kitchen', r.container);
	const join = within(menu).getByRole('menuitem', { name: 'Join its group' });
	expect(join.getAttribute('aria-disabled')).toBe('true');
	await fireEvent.click(join);
	expect(syncCalls()).toEqual([]);
});

test('busy server: the row is disabled', async () => {
	fake.app.reach = 'busy';
	const r = render(PlayerSwitcher, { onclose: () => {} });
	await waitFor(() => expect(syncRow(r)?.disabled).toBe(true));
});

test('a failure is reported, but not once another player has been selected meanwhile', async () => {
	let fail!: (e: Error) => void;
	syncReply = () => new Promise((_, rej) => (fail = rej));
	const r = await open();
	const dlg = await picker(r);
	await fireEvent.click(choices(dlg).find((b) => choiceName(b) === 'Kitchen')!);
	await waitFor(() => expect(syncCalls()).toHaveLength(1));
	fake.app.playerId = D; // the user moved on to Hall
	fail(new Error('refused'));
	await waitFor(() => expect(groupsRead()).toBe(2));
	expect(r.queryByRole('alert')).toBeNull();

	// Unchanged selection: the failure is shown.
	fake.calls.length = 0;
	fake.app.playerId = A;
	syncReply = async () => {
		throw new Error('refused');
	};
	r.unmount();
	const r2 = await open();
	const dlg2 = await picker(r2);
	await fireEvent.click(choices(dlg2).find((b) => choiceName(b) === 'Kitchen')!);
	expect((await r2.findByRole('alert')).textContent).toContain("Couldn't join");
});

test('hostile player names stay text in the row, the picker and the menus', async () => {
	const evil = '<img src=x onerror=alert(1)>';
	fake.app.players = [
		{ playerid: A, name: 'Lounge' },
		{ playerid: B, name: evil },
		{ playerid: C, name: 'Kitchen' },
	];
	groups = [`${A},${B}`];
	const r = await open();
	await waitFor(() => expect(syncRow(r)?.textContent).toContain(`In sync with ${evil}`));
	const dlg = await picker(r);
	expect(choices(dlg).map(choiceName)).toContain(`Lounge + ${evil}`);
	expect(r.container.querySelectorAll('img').length).toBe(0);
});

test('the inverse direction is labelled unmistakably', async () => {
	const r = await open();
	expect(r.getByRole('button', { name: /^Bring players into step with Lounge…/ })).toBeTruthy();
	expect(r.queryByRole('button', { name: /^Group players…/ })).toBeNull();
});
