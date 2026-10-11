<script lang="ts">
	import { page } from '$app/state';
	import { app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import AlbumBrowser from '$lib/components/AlbumBrowser.svelte';
	import ContextMenu from '$lib/components/ContextMenu.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import LibraryNote from '$lib/components/LibraryNote.svelte';

	const y = $derived.by(() => {
		try {
			return cmd.year(page.params.year);
		} catch {
			return null;
		}
	});
	let sort = $state<cmd.AlbumSort>('artistalbum');
	let showMore = $state(false);
</script>

<section class="page">
	{#if y === null}
		<p class="error">Not a year.</p>
	{:else}
		{@const year = y}
		<p class="kind muted">Year</p>
		<h1>{year === 0 ? 'No year' : year}</h1>
		<div class="actions">
			<button class="btn primary" onclick={() => app.queueUp({ year }, 'play')}><Icon name="play" size={18} />Play all</button>
			<button class="btn" onclick={() => app.queueUp({ year }, 'add')}><Icon name="plus" size={18} />Add to queue</button>
			<button class="btn" onclick={() => (showMore = true)}><Icon name="more" size={18} />More</button>
		</div>
		<LibraryNote />
		<AlbumBrowser bind:sort filter={{ year }} sorts={['album', 'artistalbum', 'new']} empty="No albums from this year." />
		{#if showMore}<ContextMenu kind="year" id={year} title={String(year)} onclose={() => (showMore = false)} />{/if}
	{/if}
</section>

<style>
	.kind {
		text-transform: uppercase;
		letter-spacing: 0.08em;
		font-size: 0.75rem;
		font-weight: 650;
		margin: 0;
	}
	h1 {
		font-size: clamp(1.8rem, 3.5vw, 2.8rem);
		margin: 0.2rem 0 0.8rem;
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 0.6rem;
		margin-bottom: 1.25rem;
	}
</style>
