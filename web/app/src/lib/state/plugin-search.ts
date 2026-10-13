// Which apps can be searched ("Search on …"), found once per session and forgotten when the
// user changes. The apps lists come through the gateway, which leaves out plugins reserved
// for admins, so a listener is never offered them.
import * as cmd from '$lib/lms/commands';
import type { AppsResult } from '$lib/lms/types';
import type { SBMenu } from '$lib/lms/slimbrowse';
import { SearchDirectory } from '$lib/search/plugins';
import { api, session } from './app.svelte';
import { resetOnUserChange } from './per-user';

/** Rows of an app's top menu read to find its search box. */
const TOP = 100;

export const pluginSearch = new SearchDirectory({
	async lists() {
		const [a, r] = await Promise.all([api.rpc<AppsResult>('', cmd.appsList()), api.rpc<AppsResult>('', cmd.radiosList()).catch(() => ({}) as AppsResult)]);
		return [a?.appss_loop ?? [], r?.radioss_loop ?? []];
	},
	open: (player, app) => api.rpc<SBMenu>(player, cmd.appMenu(app)(0, TOP)),
});
resetOnUserChange(session, () => pluginSearch.reset());
