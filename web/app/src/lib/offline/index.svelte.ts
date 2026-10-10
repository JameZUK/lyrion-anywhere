// The app's download manager and offline player, on IndexedDB (see ./store for why).
import { api, session } from '$lib/state/app.svelte';
import { coverPath } from '$lib/lms/commands';
import { DownloadManager } from './manager.svelte';
import { OfflinePlayer } from './player.svelte';
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

export const offlinePlayer = new OfflinePlayer({ store: offlineStore });
