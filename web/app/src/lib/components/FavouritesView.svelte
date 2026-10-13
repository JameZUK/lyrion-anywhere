<script lang="ts">
	// Favourites (or one folder of them): on their own page, and under Library's Favourites
	// chip (`level` 2, the top level).
	let { item, level = 1 }: { item?: string; level?: 1 | 2 } = $props();
	import BackButton from '$lib/components/BackButton.svelte';
	import { untrack } from 'svelte';
	import { api, app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { textOf } from '$lib/lms/format';
	import type { FavouritesResult, MenuItem } from '$lib/lms/types';
	import Art from '$lib/components/Art.svelte';
	import Icon from '$lib/components/Icon.svelte';

	let items = $state<MenuItem[]>([]);
	let title = $state('Favourites');
	let error = $state('');
	let loaded = $state(false);

	const folder = $derived.by(() => {
		const v = item;
		if (!v) return undefined;
		try {
			return cmd.itemId(v);
		} catch {
			return null;
		}
	});

	$effect(() => {
		const f = folder;
		untrack(() => {
			items = [];
			loaded = false;
			error = f === null ? 'Not a favourites folder.' : '';
		});
		if (f === null) return;
		let live = true;
		api.rpc<FavouritesResult>('', cmd.favourites(0, 500, f)).then(
			(r) => {
				if (!live) return;
				items = (r.loop_loop ?? []).filter((i) => typeof i.id === 'string');
				title = textOf(r.title) || 'Favourites';
				loaded = true;
			},
			(e) => live && (error = (e as Error).message),
		);
		return () => (live = false);
	});

	const isFolder = (i: MenuItem) => String(i.hasitems) === '1' && String(i.isaudio) !== '1';
	async function play(i: MenuItem, how: cmd.Placement) {
		if (!app.playerId) {
			app.error = 'Choose a player first (bottom right, or Settings › This device).';
			return;
		}
		await app.send(cmd.playFavourite(i.id!, how));
		app.queueVersion++;
	}
</script>

<div class="view">
	{#if folder}<BackButton parent="/favourites" />{/if}
	<svelte:element this={`h${level}`}>{folder ? title : 'Favourites'}</svelte:element>
	{#if error}<p class="error">{error}</p>{/if}
	{#if loaded && !items.length}<p class="muted">No favourites here yet. Add them in LMS or any LMS app.</p>{/if}
	<ul class="list">
		{#each items as i (i.id)}
			<li>
				{#if isFolder(i)}
					<a class="main" href={`#/favourites/${i.id}`}>
						<div class="icon"><Art path={cmd.imagePath(i.image, 96)} alt="" label={textOf(i.name)} /></div>
						<span class="name ellipsis">{textOf(i.name)}</span>
						<span class="chev" aria-hidden="true">›</span>
					</a>
				{:else}
					<button class="main" onclick={() => play(i, 'play')} aria-label={`Play ${textOf(i.name)}`}>
						<div class="icon"><Art path={cmd.imagePath(i.image, 96)} alt="" label={textOf(i.name)} /></div>
						<span class="name ellipsis">{textOf(i.name)}</span>
					</button>
					<button class="icon-btn" aria-label={`Play ${textOf(i.name)} next`} onclick={() => play(i, 'next')}><Icon name="playnext" size={18} /></button>
					<button class="icon-btn" aria-label={`Add ${textOf(i.name)} to the queue`} onclick={() => play(i, 'add')}><Icon name="plus" size={18} /></button>
				{/if}
			</li>
		{/each}
	</ul>
</div>

<style>
	.list {
		list-style: none;
		margin: 1rem 0 0;
		padding: 0;
	}
	li {
		display: flex;
		align-items: center;
		border-radius: var(--radius-sm);
	}
	li:hover {
		background: var(--bg-sunken);
	}
	.main {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 0.8rem;
		padding: 0.5rem 0.6rem;
		border: 0;
		background: transparent;
		text-align: left;
	}
	.icon {
		width: 44px;
		flex: none;
	}
	.name {
		flex: 1;
		font-weight: 550;
	}
	.chev {
		color: var(--fg-muted);
		font-size: 1.4rem;
	}
</style>
