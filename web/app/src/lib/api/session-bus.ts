// What one tab of this browser tells the others about the sign-in. Every tab (and the
// installed app) shares one refresh cookie, so a sign-out in one ends the sign-in in all of
// them: the others hear of it at once rather than on their next refused request.
//
// A BroadcastChannel where the browser has one; otherwise a storage event, which every other
// document of this origin sees when a key changes. Neither reaches another site, and neither
// is delivered to the tab that sent it. What arrives is checked strictly: only the one known
// message does anything.

/** A message between tabs. Only a sign-out, for now. */
export type SessionSignal = { type: 'signed-out' };

export interface SessionBus {
	post(m: SessionSignal): void;
	/** Hear the other tabs' messages; returns the way to stop. */
	listen(fn: (m: SessionSignal) => void): () => void;
}

/** The BroadcastChannel's name. */
export const BUS_NAME = 'anywhere-session';
/** The storage key the fallback writes (and removes at once). */
export const SIGNAL_KEY = 'anywhere.session.signal';

/** A message from another tab, checked: anything but a known one is null. */
export function parseSignal(v: unknown): SessionSignal | null {
	if (!v || typeof v !== 'object' || Array.isArray(v)) return null;
	return (v as { type?: unknown }).type === 'signed-out' ? { type: 'signed-out' } : null;
}

/** The parts of the browser the bus uses (the real ones by default; fakes in tests). */
export interface BusEnvironment {
	BroadcastChannel?: { new (name: string): BroadcastChannelLike } | undefined;
	storage?: Pick<Storage, 'setItem' | 'removeItem'> | undefined;
	events?: Pick<EventTarget, 'addEventListener' | 'removeEventListener'> | undefined;
}

interface BroadcastChannelLike {
	postMessage(m: unknown): void;
	addEventListener(type: 'message', fn: (e: { data: unknown }) => void): void;
	removeEventListener(type: 'message', fn: (e: { data: unknown }) => void): void;
}

function browserEnvironment(): BusEnvironment {
	const g = globalThis as typeof globalThis & { BroadcastChannel?: BusEnvironment['BroadcastChannel'] };
	let storage: BusEnvironment['storage'];
	try {
		storage = g.localStorage;
	} catch {
		storage = undefined; // storage blocked
	}
	return {
		BroadcastChannel: typeof g.BroadcastChannel === 'function' ? g.BroadcastChannel : undefined,
		storage,
		events: typeof g.addEventListener === 'function' ? g : undefined,
	};
}

/** This browser's bus between tabs, or undefined where it has neither way to send. */
export function browserBus(env: BusEnvironment = browserEnvironment()): SessionBus | undefined {
	if (env.BroadcastChannel) {
		try {
			const ch = new env.BroadcastChannel(BUS_NAME);
			return {
				post: (m) => {
					try {
						ch.postMessage(m);
					} catch {
						/* closed: the other tabs find out at their next request */
					}
				},
				listen: (fn) => {
					const on = (e: { data: unknown }) => {
						const m = parseSignal(e?.data);
						if (m) fn(m);
					};
					ch.addEventListener('message', on);
					return () => ch.removeEventListener('message', on);
				},
			};
		} catch {
			/* refused (some private modes): the fallback */
		}
	}
	const { storage, events } = env;
	if (!storage || !events) return undefined;
	return {
		post: (m) => {
			try {
				// A fresh value each time, so the same message twice is still a change.
				storage.setItem(SIGNAL_KEY, JSON.stringify({ ...m, at: Date.now(), n: Math.random() }));
				storage.removeItem(SIGNAL_KEY);
			} catch {
				/* storage blocked or full */
			}
		},
		listen: (fn) => {
			const on = (e: Event) => {
				const s = e as StorageEvent;
				if (s.key !== SIGNAL_KEY || typeof s.newValue !== 'string' || s.newValue.length > 512) return;
				let v: unknown;
				try {
					v = JSON.parse(s.newValue);
				} catch {
					return;
				}
				const m = parseSignal(v);
				if (m) fn(m);
			};
			events.addEventListener('storage', on);
			return () => events.removeEventListener('storage', on);
		},
	};
}
