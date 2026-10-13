// Play or shuffle a smart filter on the selected player (see play.ts).
import { api, app } from '$lib/state/app.svelte';
import { me } from '$lib/state/me';
import { playFilter } from './play';
import * as cmd from '$lib/lms/commands';
import type { Album, AlbumsResult } from '$lib/lms/types';

/** Resolves to '' when it played, or a message for the user. */
export async function startFilter(id: number, shuffle: boolean): Promise<string> {
	const player = app.playerId;
	if (!player) return 'Choose a player first: tap the Play on… button by the volume.';
	const msg = await playFilter({ albums: (i, from, n) => me.filterAlbums(i, from, n), send: (c) => api.rpc(player, c) }, id, shuffle);
	app.queueVersion++;
	app.soon();
	return msg;
}

/** Album details for ids from the server, in their order (asked for in lists that fit the gateway's limit). */
export async function albumsFor(ids: readonly number[]): Promise<Album[]> {
	const out: Album[] = [];
	for (const chunk of cmd.chunkIds(ids)) {
		const r = await api.rpc<AlbumsResult>('', cmd.albumsByIds(chunk));
		out.push(...cmd.inOrder(r.albums_loop ?? [], chunk));
	}
	return out;
}
