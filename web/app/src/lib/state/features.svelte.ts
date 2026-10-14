// The optional features the gateway offers (`GET /v1/me` "features"), asked for once per
// signed-in user, when something first wants to know. Until the answer comes (or if it
// never does) every feature counts as off, so an older gateway simply doesn't show them.
import { me } from './me';
import { session } from './app.svelte';

let features = $state<Record<string, boolean>>({});
let loadedFor: string | null = null;

function who(): string {
	return session.current?.user?.username ?? '';
}

/** Ask the gateway, unless already asked for this user. */
export function loadFeatures(): void {
	const user = who();
	if (loadedFor === user) return;
	loadedFor = user;
	features = {};
	me.features().then(
		(f) => {
			if (loadedFor === user) features = f;
		},
		() => {
			if (loadedFor === user) loadedFor = null; // ask again next time
		},
	);
}

export const serverFeatures = {
	/** POST /v1/players/{player}/queue/save */
	get saveQueue(): boolean {
		return features.save_queue === true;
	},
	/** For tests: forget what was learnt. */
	reset(): void {
		features = {};
		loadedFor = null;
	},
};
