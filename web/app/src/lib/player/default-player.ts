// Which player the app controls when it starts (SPEC D20: player-first).
//
// This device is the default on every device, with a per-device override in Settings ›
// This device: "This device", "Last used", or a named player. A reload of the same tab keeps
// whatever the tab was controlling (a remote selection is remembered; this device resumes
// by itself). If this browser can't play, the app falls back to the last used player and says so.

type Store = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

export type DefaultSetting = { kind: 'device' } | { kind: 'last' } | { kind: 'player'; id: string };

export const DEFAULT_SETTING: DefaultSetting = { kind: 'device' };

const MAC = /^[0-9a-f]{2}(:[0-9a-f]{2}){5}$/;
const KEY = 'anywhere.defaultPlayer';
/** This tab's selection, kept across reloads of the tab (not across tabs or visits). */
const TAB_KEY = 'anywhere.player.tab';

/** A player id as stored, lower-cased, or '' if it isn't one. */
export function cleanId(v: unknown): string {
	return typeof v === 'string' && MAC.test(v.toLowerCase()) ? v.toLowerCase() : '';
}

function safe(get: () => Store | undefined): Store | undefined {
	try {
		return get();
	} catch {
		return undefined; // storage blocked (private mode etc.)
	}
}
const local = () => safe(() => globalThis.localStorage);
const session = () => safe(() => globalThis.sessionStorage);

/** The setting is per user on this device: another account here has its own. */
function key(user: string): string {
	return `${KEY}.${encodeURIComponent(user.slice(0, 128))}`;
}

/** The stored setting, or the default ("This device") if none or malformed. */
export function loadDefault(user: string, store: Store | undefined = local()): DefaultSetting {
	try {
		const v = JSON.parse(store?.getItem(key(user)) ?? 'null') as { kind?: unknown; id?: unknown } | null;
		if (v?.kind === 'last') return { kind: 'last' };
		if (v?.kind === 'player') {
			const id = cleanId(v.id);
			if (id) return { kind: 'player', id };
		}
	} catch {
		/* malformed: the default */
	}
	return DEFAULT_SETTING;
}

export function saveDefault(user: string, s: DefaultSetting, store: Store | undefined = local()) {
	try {
		if (s.kind === 'device') store?.removeItem(key(user));
		else if (s.kind === 'last') store?.setItem(key(user), JSON.stringify({ kind: 'last' }));
		else {
			const id = cleanId(s.id);
			if (id) store?.setItem(key(user), JSON.stringify({ kind: 'player', id }));
		}
	} catch {
		/* not essential */
	}
}

export function loadTabSelection(store: Store | undefined = session()): string {
	try {
		return cleanId(store?.getItem(TAB_KEY));
	} catch {
		return '';
	}
}

export function saveTabSelection(id: string, store: Store | undefined = session()) {
	try {
		if (cleanId(id)) store?.setItem(TAB_KEY, cleanId(id));
		else store?.removeItem(TAB_KEY);
	} catch {
		/* not essential */
	}
}

export interface StartInputs {
	setting: DefaultSetting;
	/** The player this browser controlled last (any tab, any visit; may be this device), or ''. */
	lastUsed: string;
	/** The last player other than this device that this browser controlled, or ''. */
	lastRemote: string;
	/** What this tab controlled before it reloaded, or '' for a fresh start. */
	tabSelection: string;
	/** This device's own player id (from the sign-in), or '' if unknown. */
	deviceMac: string;
	/** Whether this browser can play at all. */
	deviceSupported: boolean;
}

/** Why the app is not on the player the setting asks for. */
export type StartNote = 'unsupported' | 'failed';

export type StartChoice =
	/** Start this device's player and control it once LMS lists it. */
	| { use: 'device' }
	/** Control this player (if LMS lists it; otherwise the usual pick). */
	| { use: 'player'; id: string; note?: StartNote }
	/** No preference: the first player playing, else the first listed. */
	| { use: 'auto'; note?: StartNote };

/** What to control after signing in (or reloading). Pure: see the tests. */
export function decideStart(i: StartInputs): StartChoice {
	const mac = cleanId(i.deviceMac);
	const last = cleanId(i.lastUsed);
	const remote = cleanId(i.lastRemote);
	const isDevice = (id: string) => !!id && id === mac;
	const deviceOr = (): StartChoice => {
		if (i.deviceSupported && mac) return { use: 'device' };
		// Can't play here: the last player used other than this device.
		if (remote && !isDevice(remote)) return { use: 'player', id: remote, note: 'unsupported' };
		return { use: 'auto', note: 'unsupported' };
	};

	// A reload of this tab: carry on with what it was controlling.
	const tab = cleanId(i.tabSelection);
	if (tab) return isDevice(tab) ? deviceOr() : { use: 'player', id: tab };

	switch (i.setting.kind) {
		case 'player': {
			const id = cleanId(i.setting.id);
			if (id && !isDevice(id)) return { use: 'player', id };
			return deviceOr();
		}
		case 'last':
			return last && !isDevice(last) ? { use: 'player', id: last } : deviceOr();
		default:
			return deviceOr();
	}
}

/** This device's player could not start after all: what to control instead. */
export function fallbackAfterFailure(lastRemote: string, deviceMac: string): StartChoice {
	const last = cleanId(lastRemote);
	return last && last !== cleanId(deviceMac) ? { use: 'player', id: last, note: 'failed' } : { use: 'auto', note: 'failed' };
}

export const NOTES: Record<StartNote, string> = {
	unsupported: "This browser can't play music itself, so you're controlling another player.",
	failed: "This device couldn't start playing, so you're controlling another player.",
};
