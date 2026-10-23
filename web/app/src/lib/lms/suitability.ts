// Probing apps and plugins for suitability (see `suitability` in slimbrowse.ts): a few at a
// time, remembered for a while in this tab so the Apps page doesn't re-ask on every visit.
import type { Command } from '#lib/api/rpc';
import { suitability, type SBMenu, type Suitability } from './slimbrowse';

const KEY = 'anywhere.apps.suitability';
export const TTL_MS = 10 * 60_000;
type Store = Pick<Storage, 'getItem' | 'setItem'>;
type Saved = Record<string, Suitability & { at: number }>;

function safeSession(): Store | undefined {
	try {
		return globalThis.sessionStorage;
	} catch {
		return undefined;
	}
}

function load(store: Store | undefined): Saved {
	try {
		const v = JSON.parse(store?.getItem(KEY) ?? '{}');
		return v && typeof v === 'object' && !Array.isArray(v) ? (v as Saved) : {};
	} catch {
		return {};
	}
}

const valid = (s: unknown): s is Suitability & { at: number } => {
	const o = s as Record<string, unknown>;
	return !!o && typeof o.at === 'number' && ['ok', 'setup', 'empty', 'error'].includes(o.state as string) && (o.message === undefined || typeof o.message === 'string');
};

/**
 * Judge each target (key → command opening its top menu). `onResult` is called as answers
 * arrive. Remembered answers younger than TTL_MS are reused without asking.
 */
export async function probeAll(
	targets: { key: string; command: Command }[],
	run: (c: Command) => Promise<SBMenu>,
	onResult: (key: string, s: Suitability) => void,
	opts: { concurrency?: number; store?: Store; now?: () => number } = {},
): Promise<void> {
	const store = opts.store ?? safeSession();
	const now = opts.now ?? Date.now;
	const saved = load(store);
	const queue: typeof targets = [];
	for (const t of targets) {
		const s = saved[t.key];
		if (valid(s) && now() - s.at < TTL_MS) onResult(t.key, s);
		else queue.push(t);
	}
	const worker = async () => {
		for (let t = queue.shift(); t; t = queue.shift()) {
			let s: Suitability;
			try {
				s = suitability(await run(t.command));
			} catch (e) {
				s = suitability(null, e);
			}
			saved[t.key] = { ...s, at: now() };
			onResult(t.key, s);
		}
	};
	await Promise.all(Array.from({ length: Math.max(1, opts.concurrency ?? 3) }, worker));
	try {
		store?.setItem(KEY, JSON.stringify(saved));
	} catch {
		/* not essential */
	}
}
