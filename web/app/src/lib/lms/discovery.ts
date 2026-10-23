// "Start radio" from a track, from what this LMS offers (the same rules as the native apps'
// discovery, docs/plans/native-apps.md › A6): nothing is built in or assumed.
//
// - AudioMuse-AI (an LMS plugin) adds entries to a track's context menu: a mix that plays at
//   once is run exactly as the menu offered it (the gateway only runs actions LMS offered).
// - Don't Stop The Music (built into LMS) keeps a player's queue going from a provider chosen
//   per player. Its settings menu lists the providers and the one in use; radio switches it
//   on (to the best provider) only when it is off.
//
// With a mix, radio runs it; otherwise it plays the track and lets Don't Stop The Music carry
// on. With neither there is no radio, and the app says so.
import type { Command } from '#lib/api/rpc';
import { enqueue, id as checkId, infoMenu } from './commands';
import { itemKind, needsInput, resolveAction, toCommand, type Resolved, type SBMenu } from './slimbrowse';

export const DSTM_PREF = 'plugin.dontstopthemusic:provider';
const MAX_PROVIDERS = 64;
/** Items read from a context menu when looking for AudioMuse-AI's entries. */
const INFO_ITEMS = 100;

export interface DstmProvider {
	/** LMS's token, e.g. `PLUGIN_DSTM_SIMILAR`. */
	token: string;
	name: string;
}
export interface DstmState {
	providers: DstmProvider[];
	/** The provider in use (null: off). */
	current: string | null;
}

export interface RadioPlan {
	/** AudioMuse-AI's mix, run first; null: the track itself is played. */
	mix: Resolved | null;
	mixLabel: string | null;
	/** Don't Stop The Music's provider, which keeps the queue going. */
	dstm: DstmProvider | null;
	/** Don't Stop The Music must be switched on (to `dstm`) first. */
	enableDstm: boolean;
}

/** Is this command word AudioMuse-AI's? (Its LMS plugin's words all start so.) */
export const isAudioMuse = (word: unknown) => typeof word === 'string' && word.length >= 9 && word.slice(0, 9).toLowerCase() === 'audiomuse';

const MIX_WORDS = ['mix', 'radio', 'instant', 'play similar'];
const SIMILAR_WORDS = ['similar', 'like this', 'sonic', 'related', 'alike'];
const has = (text: string, words: readonly string[]) => words.some((w) => text.toLowerCase().includes(w));
const rank = (text: string, first: readonly string[], second: readonly string[]) => (has(text, first) ? 0 : has(text, second) ? 1 : 2);

/** AudioMuse-AI's entries in a context menu: a mix that plays at once, or its own submenu. */
export function audiomuseOffers(menu: SBMenu): { mix: { label: string; action: Resolved } | null; folder: Resolved | null } {
	let mix: { r: number; label: string; action: Resolved } | null = null;
	let folder: Resolved | null = null;
	const better = (r: number) => !mix || r < mix.r;
	for (const item of Array.isArray(menu?.item_loop) ? menu.item_loop.slice(0, INFO_ITEMS) : []) {
		if (!item || typeof item !== 'object') continue;
		const go = resolveAction(menu, item, 'go');
		const play = resolveAction(menu, item, 'play');
		const action = [go, play].find((a) => a && isAudioMuse(a.cmd[0])) ?? null;
		if (!action || itemKind(menu, item) === 'search' || needsInput(action)) continue;
		const label = (typeof item.text === 'string' ? item.text : '').split('\n')[0]!.trim().slice(0, 80);
		if (!action.menu) {
			const r = rank(label, MIX_WORDS, SIMILAR_WORDS);
			if (better(r)) mix = { r, label, action };
			continue;
		}
		if (rank(label, SIMILAR_WORDS, MIX_WORDS) < 2) {
			// A list that can be played at once also makes a mix.
			if (play && !play.menu && isAudioMuse(play.cmd[0])) {
				const r = 1 + Math.min(1, rank(label, MIX_WORDS, []));
				if (better(r)) mix = { r, label, action: play };
			}
		} else if (!folder) {
			folder = action;
		}
	}
	return { mix: mix ? { label: mix.label, action: mix.action } : null, folder };
}

/** A provider token as LMS names them: letters, digits and `_`, starting with a letter. */
export const providerOk = (s: unknown): s is string => typeof s === 'string' && /^[A-Za-z][A-Za-z0-9_]{0,63}$/.test(s);

/**
 * Don't Stop The Music's settings from its menu (untrusted): only entries that set exactly the
 * provider preference to a well-formed token count. Null if the answer isn't that menu.
 */
export function dstmFrom(v: unknown): DstmState | null {
	const items = v && typeof v === 'object' ? (v as { item_loop?: unknown }).item_loop : undefined;
	if (!Array.isArray(items)) return null;
	const providers: DstmProvider[] = [];
	let current: string | null = null;
	let offSeen = false;
	for (const item of items.slice(0, MAX_PROVIDERS * 2)) {
		if (!item || typeof item !== 'object') continue;
		const it = item as Record<string, unknown>;
		const actions = it.actions && typeof it.actions === 'object' ? (it.actions as Record<string, unknown>) : {};
		const act = (actions.do ?? actions.go) as { cmd?: unknown } | undefined;
		const cmd = act && typeof act === 'object' ? act.cmd : undefined;
		if (!Array.isArray(cmd)) continue;
		const words = cmd.filter((w): w is string => typeof w === 'string');
		if (words[0] !== 'playerpref' || words[1] !== DSTM_PREF || words.length > 3) continue;
		const token = words.length === 3 ? words[2]! : typeof cmd[2] === 'number' && Number.isInteger(cmd[2]) ? String(cmd[2]) : null;
		if (token === null) continue;
		if (token === '0') {
			offSeen = true;
			continue;
		}
		if (!providerOk(token) || providers.some((p) => p.token === token)) continue;
		if (providers.length >= MAX_PROVIDERS) break;
		const raw = typeof it.text === 'string' ? it.text.replace(/[\u0000-\u001f\u007f]/g, ' ').trim().slice(0, 80) : '';
		if (it.radio === 1 || it.radio === '1') current = token;
		providers.push({ token, name: raw || token });
	}
	if (!providers.length && !offSeen) return null;
	return { providers, current };
}

/** The provider radio uses, and whether it must be switched on: the one in use if any;
 * otherwise AudioMuse-AI's, then one that finds similar music, then anything but random. */
export function chooseProvider(s: DstmState): { provider: DstmProvider; enable: boolean } | null {
	const cur = s.current ? s.providers.find((p) => p.token === s.current) : undefined;
	if (cur) return { provider: cur, enable: false };
	const score = (p: DstmProvider) => {
		const t = `${p.token} ${p.name}`.toLowerCase();
		return t.includes('audiomuse') ? 0 : t.includes('similar') || t.includes('mix') || t.includes('like') ? 1 : t.includes('random') ? 3 : 2;
	};
	const best = [...s.providers].sort((a, b) => score(a) - score(b))[0];
	return best ? { provider: best, enable: true } : null;
}

export type Rpc = (player: string, c: Command) => Promise<unknown>;

/** How "Start radio" would work for this track on this player (null: no radio here). */
export async function radioPlan(rpc: Rpc, player: string, track: number): Promise<RadioPlan | null> {
	const menu = (await rpc(player, infoMenu('track', checkId(track))(0, INFO_ITEMS))) as SBMenu;
	const offers = audiomuseOffers(menu ?? {});
	let mix = offers.mix;
	if (!mix && offers.folder) {
		// AudioMuse-AI's entries may sit one level down, in its own submenu.
		try {
			const inner = (await rpc(player, toCommand(offers.folder, { start: 0, count: INFO_ITEMS }))) as SBMenu;
			mix = audiomuseOffers(inner ?? {}).mix;
		} catch {
			/* no mix, then */
		}
	}
	let dstm: DstmState | null = null;
	try {
		dstm = dstmFrom(await rpc(player, ['dontstopthemusicsetting']));
	} catch {
		dstm = null; // refused or absent
	}
	const chosen = dstm ? chooseProvider(dstm) : null;
	if (!mix && !chosen) return null;
	return { mix: mix?.action ?? null, mixLabel: mix?.label ?? null, dstm: chosen?.provider ?? null, enableDstm: chosen?.enable ?? false };
}

/** Start radio from `track` as `plan` says: Don't Stop The Music on if needed, then the mix
 * (or the track itself). Throws on failure. */
export async function startRadio(rpc: Rpc, player: string, track: number, plan: RadioPlan): Promise<void> {
	if (plan.dstm && plan.enableDstm) {
		if (!providerOk(plan.dstm.token)) throw new RangeError('not a Don’t Stop The Music provider');
		await rpc(player, ['playerpref', DSTM_PREF, plan.dstm.token]);
	}
	if (plan.mix) {
		if (plan.mix.menu || needsInput(plan.mix)) throw new RangeError('not a mix');
		await rpc(player, toCommand(plan.mix));
	} else {
		await rpc(player, enqueue({ track: checkId(track) }, 'play'));
	}
}

/** Work out and start radio from a track. Resolves to '' when started, or why not. */
export async function radioFromTrack(rpc: Rpc, player: string, track: number): Promise<string> {
	try {
		const plan = await radioPlan(rpc, player, track);
		if (!plan) return 'Radio needs AudioMuse-AI or Don’t Stop The Music on your LMS server.';
		await startRadio(rpc, player, track, plan);
		return '';
	} catch (e) {
		return `Couldn’t start radio: ${(e as Error).message}`;
	}
}
