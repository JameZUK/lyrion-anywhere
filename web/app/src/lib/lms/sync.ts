// Sync groups (players playing in step), from LMS's `syncgroups ?` and player status.
// The gateway has already dropped players the caller may not control; this also ignores any
// id that is not a listed player, so nothing here can name or target anyone else's player.
import * as cmd from './commands';
import type { Command } from '$lib/api/rpc';
import { textOf } from './format';
import type { PlayerInfo, PlayerStatus, SyncGroupsResult } from './types';

/** Player ids in a comma-separated LMS list that are players we know (and may control). */
function known(list: unknown, players: readonly PlayerInfo[]): string[] {
	if (typeof list !== 'string' || list.length > 4096) return [];
	const ids = new Set(players.map((p) => p.playerid.toLowerCase()));
	const out: string[] = [];
	for (const raw of list.split(',')) {
		const id = raw.trim().toLowerCase();
		if (ids.has(id) && !out.includes(id)) out.push(id);
	}
	return out;
}

/** The sync groups, each as the ids of its (listed) members; groups of one are left out. */
export function syncGroupsFrom(r: SyncGroupsResult | null | undefined, players: readonly PlayerInfo[]): string[][] {
	const loop = Array.isArray(r?.syncgroups_loop) ? r.syncgroups_loop : [];
	const groups: string[][] = [];
	for (const g of loop.slice(0, 100)) {
		const ids = known(g?.sync_members, players);
		if (ids.length > 1 && !groups.some((x) => x.some((id) => ids.includes(id)))) groups.push(ids);
	}
	return groups;
}

/** The group `player` is in (including itself), or [] if it plays alone. */
export function groupOf(player: string, groups: readonly string[][]): string[] {
	return groups.find((g) => g.includes(player.toLowerCase())) ?? [];
}

/** The selected player's group from its status: itself first, then the others. */
export function groupFromStatus(player: string, s: PlayerStatus | null | undefined, players: readonly PlayerInfo[]): string[] {
	if (!s || (!s.sync_master && !s.sync_slaves)) return [];
	const ids = known([s.sync_master, s.sync_slaves].filter(Boolean).join(','), players);
	const me = player.toLowerCase();
	const others = ids.filter((id) => id !== me);
	return others.length ? [me, ...others] : [];
}

/** "Lounge + Study", "Lounge + Study + Hall", or "Lounge + 3 more". */
export function groupLabel(ids: readonly string[], players: readonly PlayerInfo[]): string {
	const name = (id: string) => textOf(players.find((p) => p.playerid.toLowerCase() === id)?.name) || 'Player';
	if (ids.length < 2) return ids.length ? name(ids[0]!) : '';
	if (ids.length <= 3) return ids.map(name).join(' + ');
	return `${name(ids[0]!)} + ${ids.length - 1} more`;
}

/** The player button's label: the selected player, or its group ("Lounge + Study"). */
export function playerLabel(player: string, s: PlayerStatus | null | undefined, players: readonly PlayerInfo[]): string {
	const group = groupFromStatus(player, s, players);
	if (group.length > 1) return groupLabel(group, players);
	return textOf(players.find((p) => p.playerid === player)?.name);
}

/**
 * What to send to put `other` in the same group as `current`, or take it out.
 *
 * `<X> sync <Y>` makes Y join X's group (leaving any group Y was in), and Y takes the queue
 * X's group plays: Y's own queue is replaced (checked against LMS 9.1). So it is always sent
 * for the selected player, naming the one that joins it. The other way round would pull the
 * selected player out of its group and onto the other player's queue.
 */
export function syncToggle(current: string, other: string, on: boolean): { player: string; cmd: Command } {
	if (!on) return { player: cmd.playerId(other), cmd: cmd.unsync() };
	return { player: cmd.playerId(current), cmd: cmd.syncWith(other) };
}
