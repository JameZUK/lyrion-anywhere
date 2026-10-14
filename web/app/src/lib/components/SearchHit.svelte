<script lang="ts">
	// One search result (Wave 2 §3.3). A track plays from its row; an album or playlist row
	// opens it and has a play button; every row has a menu with Play next and Add to queue
	// (also a long press or right-click, a swipe right or left, or N and A on the focused row).
	import { app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import type { Hit } from '$lib/search/results';
	import { rowShortcut, swipeRow } from '$lib/gestures/swipe';
	import Art from './Art.svelte';
	import Icon from './Icon.svelte';
	import RowMenu from './RowMenu.svelte';

	let { hit, onpick, top = false }: { hit: Hit; /** The user chose this result (for recent searches). */ onpick?: () => void; top?: boolean } = $props();

	let menu = $state<ReturnType<typeof RowMenu>>();
	const KIND: Record<Hit['kind'], string> = { artist: 'Artist', album: 'Album', track: 'Track', playlist: 'Playlist' };
	const sel = $derived<cmd.Selection>(
		hit.kind === 'album' ? { album: hit.id } : hit.kind === 'track' ? { track: hit.id } : hit.kind === 'playlist' ? { playlist: hit.id } : { artist: hit.id },
	);
	const href = $derived(hit.kind === 'track' ? null : cmd.link(hit.kind, hit.id));
	const cover = $derived(cmd.coverPath(hit.cover, top ? 240 : 96));

	function queue(how: cmd.Placement) {
		if (!app.canQueue) return;
		onpick?.();
		void app.queueUp(sel, how);
	}
	const keys = (e: KeyboardEvent) => rowShortcut(e, { next: () => queue('next'), add: () => queue('add') });
	function oncontextmenu(e: MouseEvent) {
		e.preventDefault();
		menu?.show();
	}
</script>

<div class="hit" class:top use:swipeRow={{ onright: () => queue('next'), onleft: () => queue('add') }} {oncontextmenu} role="group" aria-label={`${KIND[hit.kind]}: ${hit.name}`} data-hit={hit.kind}>
	{#if href}
		<a class="main" {href} onclick={() => onpick?.()} onkeydown={keys} aria-keyshortcuts="N A">
			<span class="pic" class:round={hit.kind === 'artist'}>
				{#if hit.kind === 'album' || cover}<Art path={cover} alt="" label={hit.name} />{:else}<Icon name={hit.kind === 'artist' ? 'artist' : 'queue'} size={top ? 32 : 20} />{/if}
			</span>
			<span class="text">
				<span class="name ellipsis">{hit.name}</span>
				<span class="sub ellipsis muted">{top ? KIND[hit.kind] : ''}{top && hit.sub ? ' · ' : ''}{hit.sub}</span>
			</span>
		</a>
		{#if hit.kind !== 'artist'}
			<button class="icon-btn" aria-label={`Play ${hit.name}`} onclick={() => queue('play')} disabled={!app.canQueue} data-hit-play><Icon name="play" size={18} /></button>
		{/if}
	{:else}
		<button class="main" onclick={() => queue('play')} disabled={!app.canQueue} onkeydown={keys} aria-label={`Play ${hit.name}${hit.sub ? `, ${hit.sub}` : ''}`} aria-keyshortcuts="N A" data-hit-play>
			<span class="pic">{#if cover}<Art path={cover} alt="" label={hit.name} />{:else}<Icon name="note" size={top ? 32 : 20} />{/if}</span>
			<span class="text">
				<span class="name ellipsis">{hit.name}</span>
				<span class="sub ellipsis muted">{top ? KIND[hit.kind] : ''}{top && hit.sub ? ' · ' : ''}{hit.sub}</span>
			</span>
			<span class="play" aria-hidden="true"><Icon name="play" size={16} /></span>
		</button>
	{/if}
	<RowMenu
		bind:this={menu}
		label={`More options for ${hit.name}`}
		actions={[
			...(hit.kind === 'artist' ? [{ label: 'Play', icon: 'play', run: () => queue('play'), disabled: !app.canQueue }] : []),
			{ label: 'Play next', icon: 'playnext', run: () => queue('next'), shortcut: 'N', disabled: !app.canQueue },
			{ label: 'Add to queue', icon: 'plus', run: () => queue('add'), shortcut: 'A', disabled: !app.canQueue },
		]}
	/>
</div>

<style>
	.hit {
		display: flex;
		align-items: center;
		gap: 0.2rem;
		border-radius: var(--radius-sm);
		background: var(--bg);
		transition: transform 0.15s ease;
	}
	.hit:global([data-swiping]) {
		transition: none;
	}
	.hit:hover {
		background: var(--bg-sunken);
	}
	.main {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 0.75rem;
		min-height: 44px;
		padding: 0.4rem 0.5rem;
		border: 0;
		background: transparent;
		text-align: left;
		color: inherit;
		font: inherit;
	}
	.pic {
		width: 44px;
		height: 44px;
		flex: none;
		display: grid;
		place-items: center;
		border-radius: var(--radius-sm);
		background: var(--accent-soft);
		color: var(--accent);
		overflow: hidden;
	}
	.pic.round {
		border-radius: 50%;
	}
	.top .pic {
		width: 96px;
		height: 96px;
	}
	.top .name {
		font-size: 1.35rem;
	}
	.top {
		padding: 0.6rem;
		background: var(--bg-raised);
		border: 1px solid var(--line);
	}
	.text {
		flex: 1;
		min-width: 0;
		display: grid;
	}
	.name {
		font-weight: 600;
	}
	.sub {
		font-size: 0.85rem;
	}
	.play {
		color: var(--fg-muted);
	}
	@media (prefers-reduced-motion: reduce) {
		.hit {
			transition: none;
		}
	}
</style>
