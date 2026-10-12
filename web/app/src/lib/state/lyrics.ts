// The playing track's lyrics, with this session's token.
import { LyricsState } from '$lib/lyrics/store.svelte';
import { session } from './app.svelte';

export const lyrics = new LyricsState((fn) => session.authed(fn));
