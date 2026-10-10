<script lang="ts">
	// A generic LMS menu (apps, radio, plugin and context menus), with its own back stack.
	// Only actions the menus offered are run; the gateway enforces the same.
	import { untrack } from 'svelte';
	import { api, app } from '$lib/state/app.svelte';
	import type { Command } from '$lib/api/rpc';
	import { coverPath, imagePath } from '$lib/lms/commands';
	import { textOf } from '$lib/lms/format';
	import {
		iconOf,
		itemKind,
		opensMenu,
		resolveAction,
		toCommand,
		type Resolved,
		type SBItem,
		type SBMenu,
	} from '$lib/lms/slimbrowse';
	import Art from './Art.svelte';
	import Icon from './Icon.svelte';

	let {
		start,
		initial,
		title,
		onclose,
		compact = false,
	}: {
		/** First menu: a command builder taking paging… */
		start?: (from: number, count: number) => Command;
		/** …or a menu already in hand (e.g. a group of home-menu entries). */
		initial?: SBMenu;
		title: string;
		onclose?: () => void;
		compact?: boolean;
	} = $props();

	const PAGE = 100;

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

	$effect(() => {
		const make = start;
		const menu = initial;
		const t = title;
		untrack(() => {
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
		if (r.nextWindow === 'nowPlaying') location.hash = '#/';
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
						<button class="main" onclick={() => primary(top.menu, item)}>
							{#if ic}<span class="thumb"><Art path={ic} alt="" label={textOf(item.text)} /></span>{/if}
							<span class="label">{textOf(item.text)}</span>
							{#if kind === 'folder'}<span class="chev" aria-hidden="true">›</span>{/if}
						</button>
						{#if kind === 'playable' || kind === 'folder'}
							{#if resolveAction(top.menu, item, 'add-hold') || resolveAction(top.menu, item, 'insert')}
								<button class="icon-btn" aria-label={`Play ${textOf(item.text)} next`} onclick={() => act(top.menu, item, resolveAction(top.menu, item, 'add-hold') ? 'add-hold' : 'insert', 'Play next')}><Icon name="playnext" size={18} /></button>
							{/if}
							{#if resolveAction(top.menu, item, 'add')}
								<button class="icon-btn" aria-label={`Add ${textOf(item.text)} to the queue`} onclick={() => act(top.menu, item, 'add', 'Add to queue')}><Icon name="plus" size={18} /></button>
							{/if}
						{/if}
						{#if resolveAction(top.menu, item, 'more')}
							<button class="icon-btn" aria-label={`More options for ${textOf(item.text)}`} onclick={() => act(top.menu, item, 'more', 'More')}><Icon name="more" size={18} /></button>
						{/if}
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
