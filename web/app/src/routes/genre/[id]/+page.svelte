<script lang="ts">
	import { page } from '$app/state';
	import { untrack } from 'svelte';
	import { api, app } from '$lib/state/app.svelte';
	import { library } from '$lib/state/library.svelte';
	import * as cmd from '$lib/lms/commands';
	import { textOf } from '$lib/lms/format';
	import { Pager, whenNear } from '$lib/lms/pager.svelte';
	import type { Artist, ArtistsResult, GenresResult } from '$lib/lms/types';
	import AlbumBrowser from '$lib/components/AlbumBrowser.svelte';
	import ArtistList from '$lib/components/ArtistList.svelte';
	import ContextMenu from '$lib/components/ContextMenu.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import LibraryNote from '$lib/components/LibraryNote.svelte';

	const genreId = $derived.by(() => {
		try {
			return cmd.id(page.params.id);
		} catch {
			return null;
		}
	});
	let name = $state('');
	let view = $state<'albums' | 'artists'>('albums');
	let sort = $state<cmd.AlbumSort>('album');
	let showMore = $state(false);
	let sentinel = $state<HTMLDivElement>();

	$effect(() => {
		const id = genreId;
		name = '';
		if (id === null) return;
		let live = true;
		api.rpc<GenresResult>('', cmd.genreById(id)).then(
			(r) => live && (name = textOf(r.genres_loop?.[0]?.genre) || 'Genre'),
			() => live && (name = 'Genre'),
		);
		return () => (live = false);
	});

	const artistsOf = (genre: number, lib: string | null) => async (from: number) => {
		const r = await api.rpc<ArtistsResult>('', cmd.artists(from, 200, undefined, { genre, library: lib }));
		return { rows: r.artists_loop ?? [], count: r.count };
	};
	const artists = new Pager<Artist>(async () => ({ rows: [] }), (a) => (typeof a.id === 'number' ? a.id : null));
	$effect(() => {
		const id = genreId;
		const lib = library.id;
		if (view !== 'artists' || id === null) return;
		untrack(() => void artists.reset(artistsOf(id, lib)));
	});
	$effect(() => whenNear(sentinel, () => artists.more()));
</script>

<section class="page">
	{#if genreId === null}
		<p class="error">Not a genre.</p>
	{:else}
		{@const id = genreId}
		<p class="kind muted">Genre</p>
		<h1>{name || '…'}</h1>
		<div class="actions">
			<button class="btn primary" onclick={() => app.queueUp({ genre: id }, 'play')}><Icon name="play" size={18} />Play all</button>
			<button class="btn" onclick={() => app.queueUp({ genre: id }, 'add')}><Icon name="plus" size={18} />Add to queue</button>
			<button class="btn" onclick={() => (showMore = true)}><Icon name="more" size={18} />More</button>
		</div>
		<LibraryNote />
		<div class="tabs" role="tablist" aria-label="Show">
			<button role="tab" aria-selected={view === 'albums'} class:on={view === 'albums'} onclick={() => (view = 'albums')}>Albums</button>
			<button role="tab" aria-selected={view === 'artists'} class:on={view === 'artists'} onclick={() => (view = 'artists')}>Artists</button>
		</div>
		{#if view === 'albums'}
			<AlbumBrowser bind:sort filter={{ genre: id }} empty="No albums in this genre." />
		{:else}
			{#if artists.total !== null}<p class="muted">{artists.total.toLocaleString('en-GB')} {artists.total === 1 ? 'artist' : 'artists'}</p>{/if}
			{#if artists.error}<p class="error">{artists.error}</p>{/if}
			<ArtistList artists={artists.items} />
			{#if artists.total === 0}<p class="muted">No artists in this genre.</p>{/if}
			{#if !artists.done}<div bind:this={sentinel} aria-hidden="true"></div>{/if}
		{/if}
		{#if showMore}<ContextMenu kind="genre" {id} title={name} onclose={() => (showMore = false)} />{/if}
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
		overflow-wrap: anywhere;
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 0.6rem;
		margin-bottom: 1.25rem;
	}
	.tabs {
		display: inline-flex;
		gap: 0.25rem;
		padding: 0.2rem;
		margin-bottom: 1rem;
		border: 1px solid var(--line);
		border-radius: 999px;
		background: var(--bg-raised);
	}
	.tabs button {
		border: 0;
		background: none;
		color: var(--fg-muted);
		font: inherit;
		font-weight: 600;
		padding: 0.35rem 1rem;
		border-radius: 999px;
		cursor: pointer;
	}
	.tabs button.on {
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
	}
</style>
