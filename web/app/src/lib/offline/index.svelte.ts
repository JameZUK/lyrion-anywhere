// The app's download manager and offline player, on IndexedDB (see ./store for why).
import { api, session } from '#lib/state/app.svelte';
import { coverPath } from '#lib/lms/commands';
import { ApiError } from '#lib/api/http';
import { DownloadManager, fetchInfo } from './manager.svelte';
import { HandoffOffer, type Lookup } from './handoff.svelte';
import { BadInfo } from './types';
import { OfflinePlayer } from './player.svelte';
import { PlaySync } from './plays.svelte';
import { IdbBackend, OfflineStore } from './store';
import { defaultFormat, loadPrefs, requestPersist, storageEstimate, waitForWifi } from './settings';

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
	storageEstimate: () => storageEstimate(),
});

/** Plays made offline, queued here and sent back to the server (SPEC D6). */
export const plays = new PlaySync({ store: offlineStore, authed: (fn) => session.authed(fn) });

export const offlinePlayer = new OfflinePlayer({
	store: offlineStore,
	onPlay: (user, track, startedMs, seconds) => void plays.record(user, track, startedMs, seconds),
});

/** "Continue on <player>": offered when back online with a queue of downloads (SPEC §8.4). */
export const handoff = new HandoffOffer();

/** What a downloaded track's id names now (null: LMS doesn't know it, or the answer doesn't fit). */
export const lookupTrack: Lookup = (id) =>
	session.authed(async (token) => {
		try {
			const info = await fetchInfo((...a) => fetch(...a), id, token);
			return { id: info.id, key: info.key };
		} catch (e) {
			if ((e instanceof ApiError && e.status === 404) || e instanceof BadInfo) return null;
			throw e;
		}
	});

if (typeof window !== 'undefined') {
	// Back online: send what is waiting. Leaving the page: keep the play in progress.
	window.addEventListener('online', () => plays.kick());
	// No connection at all: back online, "Continue on <player>" may be offered.
	window.addEventListener('offline', () => handoff.wentOffline());
	window.addEventListener('pagehide', () => offlinePlayer.flushPlay());
	// Wi-Fi ↔ mobile data (only where the browser reports it): hold or release the queue.
	const conn = (navigator as Navigator & { connection?: EventTarget }).connection;
	conn?.addEventListener?.('change', () => downloads.networkChanged());
}
