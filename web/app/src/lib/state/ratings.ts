// The user's ratings (Wave 2), with this session's token. Forgotten when the user changes, so
// the next person to sign in on this tab never sees them.
import { RatingsState } from '$lib/ratings/store.svelte';
import { session } from './app.svelte';
import { routine } from './me';
import { resetOnUserChange } from './per-user';

export const ratings = new RatingsState(routine);
resetOnUserChange(session, () => ratings.reset());
