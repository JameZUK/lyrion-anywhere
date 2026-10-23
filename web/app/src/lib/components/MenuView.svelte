<script lang="ts">
	// A generic LMS menu (apps, radio, plugin and context menus), with its own back stack.
	// Only actions the menus offered are run; the gateway enforces the same.
	import { untrack, type Snippet } from 'svelte';
	import { go as navigate } from '#lib/nav';
	import { api, app } from '#lib/state/app.svelte';
	import { me } from '#lib/state/me';
	import { NO_PLAYER } from '#lib/naming';
	import type { Command } from '#lib/api/rpc';
	import { ApiError } from '#lib/api/http';
	import { add as addShelf } from '#lib/home/layout';
	import { coverPath, imagePath } from '#lib/lms/commands';
	import { textOf } from '#lib/lms/format';
	import {
		iconOf,
		itemKind,
		needsInput,
		opensMenu,
		resolveAction,
		toCommand,
		type Resolved,
		type SBItem,
		type SBMenu,
	} from '#lib/lms/slimbrowse';
	import Art from './Art.svelte';
	import RowMenu, { type RowAction } from './RowMenu.svelte';
	import Icon from './Icon.svelte';

	let {
		start,
		initial,
		title,
		onclose,
		compact = false,
		root,
		intro,
	}: {
		/** First menu: a command builder taking paging… */
		start?: (from: number, count: number) => Command;
		/** …or a menu already in hand (e.g. a group of home-menu entries). */
		initial?: SBMenu;
		title: string;
		onclose?: () => void;
		compact?: boolean;
		/**
		 * The menu request (of the gateway's fixed allow-list, e.g. the home menu) that offered
		 * the first menu's items, when those came from it rather than from `start`: needed to
		 * save items as stations or pin them, which the gateway replays from the root.
		 */
		root?: Command;
		/** Shown under the heading of the first menu only (a track's rating and credits). */
		intro?: Snippet;
	} = $props();

	const PAGE = 100;

	/** What a menu source would load: the same key means the same menu. */
	function menuKey(make: ((from: number, count: number) => Command) | undefined, menu: SBMenu | undefined): string {
		try {
			return make ? `cmd:${JSON.stringify(make(0, PAGE))}` : `menu:${JSON.stringify(menu ?? null)}`;
		} catch {
			return `x:${Math.random()}`; // can't tell: treat as new
		}
	}

	interface Frame {
		title: string;
		/** Re-runnable source (for paging), or a fixed menu returned by an action. */
		source: { make: (from: number, count: number) => Command } | null;
		menu: SBMenu;
		items: SBItem[];
		total: number;
		loading: boolean;
		error: string;
	}

	let stack = $state<Frame[]>([]);
	let note = $state('');
	let inputs = $state<Record<number, string>>({});
	const top = $derived(stack[stack.length - 1]);

	const player = () => app.playerId || '';

	async function load(frame: Frame, from: number) {
		if (!frame.source) return;
		frame.loading = true;
		try {
			const r = await api.rpc<SBMenu>(player(), frame.source.make(from, PAGE));
			frame.menu = from === 0 ? r : { ...r, base: r.base ?? frame.menu.base };
			frame.items = from === 0 ? (r.item_loop ?? []) : [...frame.items, ...(r.item_loop ?? [])];
			frame.total = typeof r.count === 'number' ? r.count : frame.items.length;
			if (from === 0 && r.title && !frame.title) frame.title = textOf(r.title);
			frame.error = '';
		} catch (e) {
			frame.error = (e as Error).message;
		} finally {
			frame.loading = false;
		}
	}

	function push(t: string, source: Frame['source'], menu?: SBMenu) {
		const f: Frame = {
			title: t,
			source,
			menu: menu ?? {},
			items: menu?.item_loop ?? [],
			total: menu?.count ?? menu?.item_loop?.length ?? 0,
			loading: false,
			error: '',
		};
		stack = [...stack, f];
		const live = stack[stack.length - 1]!;
		if (!menu) void load(live, 0);
	}

	// Start over only when what the menu shows really changes. Pages re-render often (the
	// player's status is refreshed every 2 s), handing over a new but identical source each
	// time; restarting then made open menus flicker and jump back to the top.
	let shownKey = '';
	$effect(() => {
		const make = start;
		const menu = initial;
		const t = title;
		untrack(() => {
			const key = menuKey(make, menu);
			if (key === shownKey && stack.length) {
				if (stack[0] && t && stack[0].title !== t) stack[0].title = t;
				return;
			}
			shownKey = key;
			stack = [];
			if (make) push(t, { make });
			else push(t, null, menu ?? { item_loop: [] });
		});
	});

	function back() {
		if (stack.length > 1) stack = stack.slice(0, -1);
		else onclose?.();
	}

	function after(r: Resolved) {
		if (r.nextWindow === 'nowPlaying') void navigate('/');
		else if (r.nextWindow === 'parent' || r.nextWindow === 'grandparent') back();
		else if (r.nextWindow === 'refresh' && top) void load(top, 0);
		app.soon();
		app.queueVersion++;
	}

	async function run(item: SBItem, r: Resolved, label: string, input?: string) {
		note = '';
		if (opensMenu(r)) {
			const src = { make: (from: number, count: number) => toCommand(r, { start: from, count }, input) };
			push(textOf(item.text) || label, src);
			return;
		}
		try {
			const res = await api.rpc<SBMenu>(player(), toCommand(r, undefined, input));
			if (Array.isArray(res.item_loop) && res.item_loop.length) {
				push(textOf(res.title) || textOf(item.text) || label, null, res); // an action that returns a menu
				return;
			}
			note = `${label}: done`;
			after(r);
		} catch (e) {
			note = (e as Error).message;
		}
	}

	function act(menu: SBMenu, item: SBItem, name: string, label: string) {
		const r = resolveAction(menu, item, name);
		if (r) void run(item, r, label);
	}

	function primary(menu: SBMenu, item: SBItem) {
		const kind = itemKind(menu, item);
		const play = resolveAction(menu, item, 'play');
		const go = resolveAction(menu, item, 'go');
		if (kind === 'playable' && play) return run(item, play, 'Play');
		if (go) return run(item, go, textOf(item.text) || 'Open');
	}

	function search(menu: SBMenu, item: SBItem, i: number) {
		const go = resolveAction(menu, item, 'go');
		const text = (inputs[i] ?? '').trim();
		if (go && text) void run(item, go, textOf(item.text) || 'Search', text);
	}

	// --- Save as station / Pin to Home ------------------------------------------------------
	// The gateway stores a reference to what the menu offered: the menu requests from the root
	// to this item, and the item's own actions; it replays and re-checks them every time.

	/** Most menu requests from the root to an item the gateway keeps. */
	const MAX_STEPS = 4;

	/** The menu requests from the root to the open menu, with the page holding item `i`. */
	function steps(i: number): Command[] | null {
		const out: Command[] = root ? [root] : [];
		for (const [n, f] of stack.entries()) {
			if (!f.source) {
				// Only the first menu may come without a request, and only from `root`.
				if (n === 0 && root) continue;
				return null;
			}
			const from = n === stack.length - 1 ? Math.floor(i / PAGE) * PAGE : 0;
			out.push(f.source.make(from, PAGE));
		}
		return out.length && out.length <= MAX_STEPS ? out : null;
	}

	/** A command the item runs (not a menu it opens, never one needing typed text). */
	function command(menu: SBMenu, item: SBItem, name: string): Command | null {
		const r = resolveAction(menu, item, name);
		return r && !opensMenu(r) && !needsInput(r) ? toCommand(r) : null;
	}

	function stationOf(menu: SBMenu, item: SBItem, i: number) {
		const play = command(menu, item, 'play') ?? command(menu, item, 'go');
		const s = play ? steps(i) : null;
		if (!play || !s) return null;
		const add = command(menu, item, 'add');
		return { kind: 'menu' as const, steps: s, play, ...(add ? { add } : {}) };
	}

	function pinOf(menu: SBMenu, item: SBItem, i: number): Command[] | null {
		const go = resolveAction(menu, item, 'go');
		if (!go || !opensMenu(go) || needsInput(go)) return null;
		const s = steps(i);
		return s && s.length < MAX_STEPS ? [...s, toCommand(go, { start: 0, count: PAGE })] : null;
	}

	const why = (e: unknown) => (e instanceof ApiError && e.status === 422 ? `This can’t be saved: ${e.message}` : (e as Error).message);

	async function saveStation(menu: SBMenu, item: SBItem, i: number) {
		const spec = stationOf(menu, item, i);
		if (!spec) return;
		if (!player()) {
			note = NO_PLAYER;
			return;
		}
		try {
			const name = (textOf(item.text) || 'Station').slice(0, 64);
			await me.createStation(name, spec, player());
			note = `Saved “${name}” as a station`;
		} catch (e) {
			note = why(e);
		}
	}

	async function pin(menu: SBMenu, item: SBItem, i: number) {
		const s = pinOf(menu, item, i);
		if (!s) return;
		if (!player()) {
			note = NO_PLAYER;
			return;
		}
		try {
			const name = (textOf(item.text) || 'Menu').slice(0, 64);
			const pinned = await me.createMenu(name, s, player());
			const home = await me.home();
			await me.saveHome(addShelf(home, { type: 'menu', id: pinned.id }));
			note = `Pinned “${name}” to Home`;
		} catch (e) {
			note = why(e);
		}
	}

	/**
	 * A row's ⋯ (Wave 3.5: one trailing control per row): play next, add to the queue, save as a
	 * station, pin to Home and the item's own "more", where the menu offers each.
	 */
	function rowActions(menu: SBMenu, item: SBItem, i: number, kind: string): RowAction[] {
		const out: RowAction[] = [];
		const off = !app.canQueue;
		if (kind === 'playable' || kind === 'folder') {
			if (resolveAction(menu, item, 'add-hold') || resolveAction(menu, item, 'insert'))
				out.push({ label: 'Play next', icon: 'playnext', run: () => act(menu, item, resolveAction(menu, item, 'add-hold') ? 'add-hold' : 'insert', 'Play next'), disabled: off });
			if (resolveAction(menu, item, 'add')) out.push({ label: 'Add to queue', icon: 'plus', run: () => act(menu, item, 'add', 'Add to queue'), disabled: off });
		}
		if ((kind === 'playable' || kind === 'action') && stationOf(menu, item, i)) out.push({ label: 'Save as a station', icon: 'radio', run: () => void saveStation(menu, item, i) });
		if (kind === 'folder' && pinOf(menu, item, i)) out.push({ label: 'Pin to Home', icon: 'shelves', run: () => void pin(menu, item, i) });
		if (resolveAction(menu, item, 'more')) out.push({ label: 'More…', icon: 'more', run: () => act(menu, item, 'more', 'More') });
		return out;
	}

	function icon(item: SBItem): string | null {
		const v = iconOf(item);
		return imagePath(v, 96) ?? coverPath(v, 96);
	}
</script>

<section class="menu" class:compact>
	<header>
		{#if stack.length > 1 || onclose}
			<button class="icon-btn" onclick={back} aria-label={stack.length > 1 ? 'Back' : 'Close'}>
				<Icon name={stack.length > 1 ? 'back' : 'close'} />
			</button>
		{/if}
		<h2 class="ellipsis">{top?.title || title}</h2>
	</header>
	{#if intro && stack.length <= 1}{@render intro()}{/if}
	{#if note}<p class="note" role="status">{note}</p>{/if}
	{#if top}
		{#if top.error}<p class="error">{top.error}</p>{/if}
		<ul>
			{#each top.items as item, i (i)}
				{@const kind = itemKind(top.menu, item)}
				{@const ic = icon(item)}
				<li class={kind}>
					{#if kind === 'search'}
						<form class="search" onsubmit={(e) => { e.preventDefault(); search(top.menu, item, i); }}>
							<Icon name="search" size={18} />
							<label class="sr-only" for={`mv-in-${i}`}>{textOf(item.text)}</label>
							<input id={`mv-in-${i}`} bind:value={inputs[i]} placeholder={textOf(item.text) || 'Search'} maxlength="200" autocomplete="off" />
							<button class="btn" type="submit">Go</button>
						</form>
					{:else if kind === 'text'}
						<p class="text">{textOf(item.text, 4000)}</p>
					{:else}
						<button class="main" onclick={() => primary(top.menu, item)} disabled={!app.canQueue}>
							{#if ic}<span class="thumb"><Art path={ic} alt="" label={textOf(item.text)} /></span>{/if}
							<span class="label">{textOf(item.text)}</span>
							{#if kind === 'folder'}<span class="chev" aria-hidden="true">›</span>{/if}
							{#if kind === 'action'}<span class="run" aria-hidden="true"><Icon name="play" size={14} /></span>{/if}
						</button>
						{@const acts = rowActions(top.menu, item, i, kind)}
						{#if acts.length}<RowMenu label={`More options for ${textOf(item.text)}`} actions={acts} />{/if}
					{/if}
				</li>
			{:else}
				{#if !top.loading && !top.error}<li class="empty muted">Nothing here.</li>{/if}
			{/each}
		</ul>
		{#if top.loading}<p class="muted">Loading…</p>{/if}
		{#if top.source && !top.loading && top.items.length < top.total}
			<button class="btn more" onclick={() => load(top, top.items.length)}>Show more ({top.total - top.items.length} left)</button>
		{/if}
	{/if}
</section>

<style>
	header {
		display: flex;
		align-items: center;
		gap: 0.4rem;
		margin-bottom: 0.75rem;
	}
	h2 {
		margin: 0;
		font-size: 1.35rem;
	}
	.compact h2 {
		font-size: 1.1rem;
	}
	ul {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	li {
		display: flex;
		align-items: center;
		border-radius: var(--radius-sm);
	}
	li:not(.text):not(.search):hover {
		background: var(--bg-sunken);
	}
	.main {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 0.75rem;
		min-height: 44px;
		padding: 0.55rem 0.6rem;
		border: 0;
		background: transparent;
		text-align: left;
	}
	.thumb {
		width: 40px;
		flex: none;
	}
	.label {
		flex: 1;
		min-width: 0;
		overflow-wrap: anywhere;
	}
	.chev {
		color: var(--fg-muted);
		font-size: 1.3rem;
	}
	/* Runs something (e.g. makes a mix) rather than opening a menu. */
	.run {
		display: inline-grid;
		place-items: center;
		width: 26px;
		height: 26px;
		border-radius: 50%;
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
	}
	.text {
		margin: 0.3rem 0.6rem;
		white-space: pre-line;
		color: var(--fg);
	}
	.search {
		flex: 1;
		display: flex;
		align-items: center;
		gap: 0.5rem;
		padding: 0.4rem 0.6rem;
		color: var(--fg-muted);
	}
	.search input {
		flex: 1;
		min-width: 0;
		padding: 0.5em 0.7em;
		border: 1px solid var(--line);
		border-radius: 999px;
		background: var(--bg-raised);
		color: var(--fg);
	}
	.note {
		padding: 0.4rem 0.7rem;
		border-radius: var(--radius-sm);
		background: var(--accent-soft);
		margin: 0 0 0.6rem;
	}
	.more {
		margin-top: 0.75rem;
	}
	.empty {
		padding: 0.6rem;
	}
</style>
