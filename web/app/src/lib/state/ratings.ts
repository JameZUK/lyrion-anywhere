// The user's ratings (Wave 2), with this session's token.
import { RatingsState } from '$lib/ratings/store.svelte';
import { routine } from './me';

export const ratings = new RatingsState(routine);
