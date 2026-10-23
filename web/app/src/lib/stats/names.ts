// Albums and tracks in the stats come as ids only: their titles and art are loaded from LMS
// by id, as for Home shelves. Ids LMS no longer knows (after a rescan, say) stay unnamed.
import { api } from '#lib/state/app.svelte';
import * as cmd from '#lib/lms/commands';
import type { Album, AlbumsResult, TitlesResult, Track } from '#lib/lms/types';

export interface StatsNames {
	albums: Map<number, Album>;
	tracks: Map<number, Track>;
}

export const noNames = (): StatsNames => ({ albums: new Map(), tracks: new Map() });

/** Titles for these album and track ids; whatever fails to load is simply left out. */
export async function loadNames(albumIds: readonly number[], trackIds: readonly number[]): Promise<StatsNames> {
	const out = noNames();
	const albumWork = cmd.chunkIds([...new Set(albumIds)]).map(async (part) => {
		try {
			const r = await api.rpc<AlbumsResult>('', cmd.albumsByIds(part));
			for (const a of cmd.inOrder(r?.albums_loop ?? [], part)) out.albums.set(a.id, a);
		} catch {
			/* left unnamed */
		}
	});
	const trackWork = cmd.chunkIds([...new Set(trackIds)]).map(async (part) => {
		try {
			const r = await api.rpc<TitlesResult>('', cmd.tracksByIds(part));
			for (const t of cmd.inOrder(r?.titles_loop ?? [], part)) if (typeof t.id === 'number') out.tracks.set(t.id, t);
		} catch {
			/* left unnamed */
		}
	});
	await Promise.all([...albumWork, ...trackWork]);
	return out;
}
