// The signed-in user's own data on the gateway, with this session's token.
import { MeApi } from '$lib/api/me';
import { session } from './app.svelte';

export const me = new MeApi((fn) => session.authed(fn));
