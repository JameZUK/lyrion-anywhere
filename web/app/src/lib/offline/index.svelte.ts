// The app's download manager and offline player, on IndexedDB (see ./store for why).
import { api, session } from '$lib/state/app.svelte';
import { coverPath } from '$lib/lms/commands';
import { DownloadManager } from './manager.svelte';
import { OfflinePlayer } from './player.svelte';
import { PlaySync } from './plays.svelte';
import { IdbBackend, OfflineStore } from './store';
import { defaultFormat, loadPrefs, requestPersist, waitForWifi } from './settings';

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
	waitForWifi: () => waitForWifi(),
	capBytes: () => loadPrefs().capBytes,
	storageEstimate: async () => (await globalThis.navigator?.storage?.estimate?.()) ?? {},
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
	// Wi-Fi ↔ mobile data (only where the browser reports it): hold or release the queue.
	const conn = (navigator as Navigator & { connection?: EventTarget }).connection;
	conn?.addEventListener?.('change', () => downloads.networkChanged());
}
