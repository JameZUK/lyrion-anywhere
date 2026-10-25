// What the gateway is (`GET /v1/me`): its player-protocol version and the optional features
// it offers, asked for once per signed-in user, when something first wants to know. Until the
// answer comes (or if it never does) every feature counts as off, so an older gateway simply
// doesn't show them, and the protocol is unknown (null).
import { me } from './me';
import { session } from './app.svelte';
import type { ServerInfo } from '#lib/api/me';

let features = $state<Record<string, boolean>>({});
let protocol = $state<number | null>(null);
let loadedFor: string | null = null;
let loading: Promise<ServerInfo | null> | null = null;

function who(): string {
	return session.current?.user?.username ?? '';
}

/**
 * Ask the gateway, unless already asked for this user. Resolves to what it said, or null when
 * it couldn't be asked (no connection): then it is asked again next time.
 */
export function loadServerInfo(): Promise<ServerInfo | null> {
	const user = who();
	if (loadedFor === user && loading) return loading;
	loadedFor = user;
	features = {};
	protocol = null;
	loading = me.info().then(
		(info) => {
			if (loadedFor === user) {
				features = info.features;
				protocol = info.protocol;
			}
			return info;
		},
		() => {
			if (loadedFor === user) {
				loadedFor = null; // ask again next time
				loading = null;
			}
			return null;
		},
	);
	return loading;
}

/** Ask the gateway for its features (see loadServerInfo). */
export function loadFeatures(): void {
	void loadServerInfo();
}

export const serverFeatures = {
	/** POST /v1/players/{player}/queue/save */
	get saveQueue(): boolean {
		return features.save_queue === true;
	},
	/** The gateway's player protocol, once known (null: not known, or an older gateway). */
	get protocol(): number | null {
		return protocol;
	},
	/** For tests: forget what was learnt. */
	reset(): void {
		features = {};
		protocol = null;
		loadedFor = null;
		loading = null;
	},
};
