// The library search, kept while the user goes to a result and back (forgotten when the user
// changes). One `search … extended:1` gives artists, albums and tracks; saved playlists come
// from `playlists search:` (if that fails, the rest still shows).
import * as cmd from '#lib/lms/commands';
import { groupResults, type Groups } from '#lib/search/results';
import { SearchState } from '#lib/search/store.svelte';
import { api, session } from './app.svelte';
import { resetOnUserChange } from './per-user';

/** Rows of each type shown under All (each type pages on its own chip). */
export const SEARCH_ROWS = 20;

export const search = new SearchState<Groups>(async (q) => {
	const [all, lists] = await Promise.all([
		api.rpc<unknown>('', cmd.searchAll(q, SEARCH_ROWS)),
		api.rpc<unknown>('', cmd.searchPlaylists(q, 0, SEARCH_ROWS)).catch(() => ({})),
	]);
	return groupResults(all, lists);
});
resetOnUserChange(session, () => search.reset());
