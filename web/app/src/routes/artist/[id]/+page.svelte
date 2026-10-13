<script lang="ts">
	import BackButton from '$lib/components/BackButton.svelte';
	import { useDetailTitle } from '$lib/title.svelte';
	import { page } from '$app/state';
	import { api, app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { textOf } from '$lib/lms/format';
	import type { Album, AlbumsResult } from '$lib/lms/types';
	import AlbumGrid from '$lib/components/AlbumGrid.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import ContextMenu from '$lib/components/ContextMenu.svelte';

	let showMore = $state(false);

	let albums = $state<Album[]>([]);
	let name = $state('');
	let error = $state('');

	const artistId = $derived.by(() => {
		try {
			return cmd.id(page.params.id);
		} catch {
			return null;
		}
	});

	$effect(() => {
		const id = artistId;
		albums = [];
		name = '';
		error = id === null ? 'Not an artist.' : '';
		if (id === null) return;
		let live = true;
		api.rpc<AlbumsResult>('', cmd.artistAlbums(id)).then(
			(r) => {
				if (!live) return;
				albums = r.albums_loop ?? [];
				// LMS lists the album artist; use the most common one as the heading.
				const counts = new Map<string, number>();
				for (const a of albums) if (a.artist) counts.set(a.artist, (counts.get(a.artist) ?? 0) + 1);
				name = [...counts.entries()].sort((x, y) => y[1] - x[1])[0]?.[0] ?? 'Artist';
			},
			(e) => live && (error = (e as Error).message),
		);
		return () => (live = false);
	});
	useDetailTitle(() => textOf(name));
</script>

<section class="page">
	<BackButton parent="/artists" />
	{#if error}
		<p class="error">{error}</p>
	{:else if artistId !== null}
		{@const id = artistId}
		<p class="kind muted">Artist</p>
		<h1>{textOf(name) || '…'}</h1>
		<div class="actions">
			<button class="btn primary" onclick={() => app.queueUp({ artist: id }, 'play')} disabled={!albums.length}><Icon name="play" size={18} />Play all</button>
			<button class="btn" onclick={() => app.queueUp({ artist: id }, 'add')} disabled={!albums.length}><Icon name="plus" size={18} />Add to queue</button>
			<button class="btn" onclick={() => (showMore = true)}><Icon name="more" size={18} />More</button>
		</div>
		<h2>{albums.length} {albums.length === 1 ? 'album' : 'albums'}</h2>
		<AlbumGrid {albums} showArtist={false} />
		{#if showMore}<ContextMenu kind="artist" {id} title={textOf(name)} onclose={() => (showMore = false)} />{/if}
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
		gap: 0.6rem;
		margin-bottom: 2rem;
	}
	h2 {
		font-size: 1rem;
		color: var(--fg-muted);
	}
</style>
