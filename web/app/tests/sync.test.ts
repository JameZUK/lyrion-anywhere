// Sync groups: only players the user may control (those listed) are named or targeted, and
// the right player is told to join the right group.
import { groupFromStatus, groupLabel, groupOf, playerLabel, syncGroupsFrom, syncToggle } from '$lib/lms/sync';
import type { PlayerInfo } from '$lib/lms/types';

const A = '02:00:00:00:00:0a';
const B = '02:00:00:00:00:0b';
const C = '02:00:00:00:00:0c';
const D = '02:00:00:00:00:0d';
const HIDDEN = '02:00:00:00:00:ff';
const players: PlayerInfo[] = [
	{ playerid: A, name: 'Lounge' },
	{ playerid: B, name: 'Study' },
	{ playerid: C, name: 'Kitchen' },
	{ playerid: D, name: 'Hall' },
];

test('groups keep only listed players, and drop groups of one', () => {
	const groups = syncGroupsFrom(
		{
			syncgroups_loop: [
				{ sync_members: `${A},${B.toUpperCase()},${HIDDEN}` },
				{ sync_members: `${C},${HIDDEN}` }, // alone, as far as this user can see
				{ sync_members: `../x,<b>,${D}` },
			],
		},
		players,
	);
	expect(groups).toEqual([[A, B]]);
	expect(groupOf(B, groups)).toEqual([A, B]);
	expect(groupOf(C, groups)).toEqual([]);
});

test('malformed sync data is ignored, never thrown on', () => {
	for (const bad of [null, undefined, {}, { syncgroups_loop: 'x' }, { syncgroups_loop: [null, 5, { sync_members: 7 }] }, { syncgroups_loop: [{ sync_members: 'x'.repeat(10_000) }] }]) {
		expect(syncGroupsFrom(bad as never, players)).toEqual([]);
	}
	expect(groupFromStatus(A, { sync_master: 42 as unknown as string }, players)).toEqual([]);
});

test('the selected player’s group from its status, itself first', () => {
	expect(groupFromStatus(B, { sync_master: A, sync_slaves: `${B},${HIDDEN}` }, players)).toEqual([B, A]);
	expect(groupFromStatus(A, { mode: 'play' }, players)).toEqual([]);
	expect(groupFromStatus(A, null, players)).toEqual([]);
	// Only itself is visible: not shown as a group.
	expect(groupFromStatus(A, { sync_master: A, sync_slaves: HIDDEN }, players)).toEqual([]);
});

test('labels: "Lounge + Study", up to three names, then a count', () => {
	expect(groupLabel([A, B], players)).toBe('Lounge + Study');
	expect(groupLabel([A, B, C], players)).toBe('Lounge + Study + Kitchen');
	expect(groupLabel([A, B, C, D], players)).toBe('Lounge + 3 more');
	expect(groupLabel([A], players)).toBe('Lounge');
	expect(groupLabel([], players)).toBe('');
	expect(playerLabel(B, { sync_master: A, sync_slaves: B }, players)).toBe('Study + Lounge');
	expect(playerLabel(B, { mode: 'stop' }, players)).toBe('Study');
	expect(playerLabel('', null, players)).toBe('');
});

test('toggling sends the joining player sync <other>, and sync - to leave', () => {
	// Playing alone: the selected player joins the other's group.
	expect(syncToggle(A, C, [], true)).toEqual({ player: A, cmd: ['sync', C] });
	// Already grouped: the other player joins this group.
	expect(syncToggle(A, C, [[A, B]], true)).toEqual({ player: C, cmd: ['sync', A] });
	// Taking a player out: that player leaves.
	expect(syncToggle(A, B, [[A, B]], false)).toEqual({ player: B, cmd: ['sync', '-'] });
	// Ids that aren't player ids are refused before anything is sent.
	expect(() => syncToggle(A, 'x; rescan', [], true)).toThrow(RangeError);
	expect(() => syncToggle('-', B, [], true)).toThrow(RangeError);
});
