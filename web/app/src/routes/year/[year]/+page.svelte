<script lang="ts">
	import BackButton from '#lib/components/BackButton.svelte';
	import { useDetailTitle } from '#lib/title.svelte';
	import { page } from '$app/state';
	import * as cmd from '#lib/lms/commands';
	import AlbumBrowser from '#lib/components/AlbumBrowser.svelte';
	import ContextMenu from '#lib/components/ContextMenu.svelte';
	import PageActions from '#lib/components/PageActions.svelte';
	import LibraryNote from '#lib/components/LibraryNote.svelte';

	const y = $derived.by(() => {
		try {
			return cmd.year(page.params.year);
		} catch {
			return null;
		}
	});
	let sort = $state<cmd.AlbumSort>('artistalbum');
	let showMore = $state(false);
	useDetailTitle(() => y === null ? '' : y === 0 ? 'No year' : String(y));
</script>

<section class="page">
	<BackButton parent="/years" />
	{#if y === null}
		<p class="error">Not a year.</p>
	{:else}
		{@const year = y}
		<p class="kind muted">Year</p>
		<h1>{year === 0 ? 'No year' : year}</h1>
		<PageActions sel={{ year }} title={year === 0 ? 'No year' : String(year)} next={false} onmore={() => (showMore = true)} />
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
	section > :global(.page-actions) {
		margin: 0 0 1.25rem;
	}
</style>
