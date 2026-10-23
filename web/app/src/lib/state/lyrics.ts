// The playing track's lyrics, with this session's token (forgotten when the user changes).
import { LyricsState } from '#lib/lyrics/store.svelte';
import { session } from './app.svelte';
import { resetOnUserChange } from './per-user';

export const lyrics = new LyricsState((fn) => session.authed(fn));
resetOnUserChange(session, () => lyrics.reset());
