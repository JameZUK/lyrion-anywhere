// The app's download manager and offline player, on IndexedDB (see ./store for why).
import { api, session } from '$lib/state/app.svelte';
import { coverPath } from '$lib/lms/commands';
import { DownloadManager } from './manager.svelte';
import { OfflinePlayer } from './player.svelte';
import { PlaySync } from './plays.svelte';
import { IdbBackend, OfflineStore } from './store';
import { defaultFormat, requestPersist } from './settings';

export const offlineStore = new OfflineStore(new IdbBackend());

export const downloads = new DownloadManager({
	store: offlineStore,
	authed: (fn) => session.authed(fn),
	cover: async (id) => {
		const path = coverPath(id, 300);
		return path ? api.artBlob(path) : null;
	},
	defaultFormat: () => defaultFormat(),
	persist: () => requestPersist(),
});

/** Plays made offline, queued here and sent back to the server (SPEC D6). */
export const plays = new PlaySync({ store: offlineStore, authed: (fn) => session.authed(fn) });

export const offlinePlayer = new OfflinePlayer({
	store: offlineStore,
	onPlay: (user, track, startedMs, seconds) => void plays.record(user, track, startedMs, seconds),
});

if (typeof window !== 'undefined') {
	// Back online: send what is waiting. Leaving the page: keep the play in progress.
	window.addEventListener('online', () => plays.kick());
	window.addEventListener('pagehide', () => offlinePlayer.flushPlay());
}
