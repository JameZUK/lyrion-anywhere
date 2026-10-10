<script lang="ts">
	import { untrack } from 'svelte';
	import { api, app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import type { Album, AlbumsResult } from '$lib/lms/types';
	import AlbumGrid from '$lib/components/AlbumGrid.svelte';

	const PAGE = 120;
	const SORTS: [cmd.AlbumSort, string][] = [
		['album', 'Title'],
		['artistalbum', 'Artist'],
		['yearalbum', 'Year'],
		['new', 'Recently added'],
		['random', 'Random'],
	];
	let sort = $state<cmd.AlbumSort>('album');
	let albums = $state<Album[]>([]);
	let total = $state<number | null>(null);
	let loading = false;
	let generation = 0;
	let error = $state('');

	async function more() {
		if (loading || (total !== null && albums.length >= total)) return;
		loading = true;
		const g = generation;
		try {
			const r = await api.rpc<AlbumsResult>('', cmd.albums(albums.length, PAGE, sort));
			if (g !== generation) return;
			albums = [...albums, ...(r.albums_loop ?? [])];
			total = r.count ?? albums.length;
		} catch (e) {
			error = (e as Error).message;
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		void sort;
		void app.libraryVersion;
		untrack(() => {
			generation++;
			albums = [];
			total = null;
			loading = false;
			void more();
		});
	});
</script>

<section class="page">
	<header>
		<h1>Albums</h1>
		<label class="sort">
			<span class="muted">Sort</span>
			<select bind:value={sort}>
				{#each SORTS as [v, label] (v)}<option value={v}>{label}</option>{/each}
			</select>
		</label>
	</header>
	{#if total !== null}<p class="muted count">{total.toLocaleString('en-GB')} albums</p>{/if}
	{#if error}<p class="error">{error}</p>{/if}
	<AlbumGrid {albums} more={total === null || albums.length < total ? more : null} />
</section>

<style>
	header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 1rem;
	}
	h1 {
		margin: 0;
	}
	.sort {
		display: flex;
		align-items: center;
		gap: 0.5rem;
	}
	select {
		padding: 0.4em 0.6em;
		border-radius: var(--radius-sm);
		border: 1px solid var(--line);
		background: var(--bg-raised);
	}
	.count {
		margin: 0.25rem 0 1.25rem;
	}
</style>
