// Playing a smart filter (wave2-design.md §5.6): its first albums, up to 50, loaded through
// the usual album play path (the first replaces the queue and plays, the rest are added).
// Shuffle turns on LMS's track shuffle first, so the albums load shuffled.
import type { Command } from '#lib/api/rpc';
import { enqueue, shuffle as shuffleCmd } from '#lib/lms/commands';
import { FILTER_LIMITS, validId } from '#lib/api/filters';

/** The commands that play these albums (ids from the server; at most 50 are used). */
export function filterPlayCommands(albumIds: readonly number[], shuffle: boolean): Command[] {
	const ids = [...new Set(albumIds.filter(validId))].slice(0, FILTER_LIMITS.playMax);
	if (!ids.length) return [];
	const out: Command[] = [];
	if (shuffle) out.push(shuffleCmd(1));
	ids.forEach((album, i) => out.push(enqueue({ album }, i === 0 ? 'play' : 'add')));
	return out;
}

export interface FilterPlayDeps {
	/** The filter's albums (as `MeApi.filterAlbums`). */
	albums: (id: number, from: number, n: number) => Promise<{ albums: number[] }>;
	/** Send one command to the selected player; throws on failure. */
	send: (c: Command) => Promise<unknown>;
}

/**
 * Play or shuffle a filter on the selected player. Resolves to a message for the user: '' when
 * it played, else why not. Stops at the first command that fails.
 */
export async function playFilter(deps: FilterPlayDeps, id: number, shuffle: boolean): Promise<string> {
	let ids: number[];
	try {
		ids = (await deps.albums(id, 0, FILTER_LIMITS.playMax)).albums;
	} catch (e) {
		return `Couldn’t read the filter: ${(e as Error).message}`;
	}
	const commands = filterPlayCommands(ids, shuffle);
	if (!commands.length) return 'No albums match this smart playlist.';
	for (const c of commands) {
		try {
			await deps.send(c);
		} catch (e) {
			return `Couldn’t play the filter: ${(e as Error).message}`;
		}
	}
	return '';
}
