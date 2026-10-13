// The library search, kept while the user goes to a result and back (forgotten when the user
// changes).
import * as cmd from '$lib/lms/commands';
import type { SearchResult } from '$lib/lms/types';
import { SearchState } from '$lib/search/store.svelte';
import { api, session } from './app.svelte';
import { resetOnUserChange } from './per-user';

export const search = new SearchState((q) => api.rpc<SearchResult>('', cmd.search(q, 30)));
resetOnUserChange(session, () => search.reset());
