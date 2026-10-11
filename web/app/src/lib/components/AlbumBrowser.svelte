<script lang="ts" module>
	import type { AlbumSort } from '$lib/lms/commands';
	export const SORT_LABELS: Record<AlbumSort, string> = {
		album: 'Title',
		artistalbum: 'Artist',
		yearalbum: 'Year',
		new: 'Recently added',
		random: 'Random',
	};
	/** Album orders offered on album lists. */
	export const ALBUM_SORTS: AlbumSort[] = ['album', 'artistalbum', 'yearalbum', 'new'];
</script>

<script lang="ts">
	// An album list from LMS: one filtered `albums` query, paged as it scrolls (or a single
	// batch, for random albums), in the chosen virtual library, with a count and empty state.
	import { untrack } from 'svelte';
	import { api, app } from '$lib/state/app.svelte';
	import { library } from '$lib/state/library.svelte';
	import * as cmd from '$lib/lms/commands';
	import type { Album, AlbumsResult } from '$lib/lms/types';
	import AlbumGrid from './AlbumGrid.svelte';

	let {
		filter = {},
		sort = $bindable('album'),
		sorts = ALBUM_SORTS,
		paged = true,
		pageSize = 120,
		version = 0,
		showArtist = true,
		empty = 'No albums here.',
	}: {
		filter?: cmd.Filter;
		sort?: AlbumSort;
		/** Orders to offer; none for a fixed order. */
		sorts?: AlbumSort[] | null;
		/** Page as the list scrolls; otherwise one batch of `pageSize`. */
		paged?: boolean;
		pageSize?: number;
		/** Bump to load the list again (e.g. "Shuffle again"). */
		version?: number;
		showArtist?: boolean;
		empty?: string;
	} = $props();

	let albums = $state<Album[]>([]);
	let total = $state<number | null>(null);
	let error = $state('');
	let loading = false;
	let generation = 0;

	async function more() {
		if (loading || (total !== null && albums.length >= total)) return;
		loading = true;
		const g = generation;
		try {
			const r = await api.rpc<AlbumsResult>(
				'',
				cmd.albums(albums.length, pageSize, sort, undefined, { ...filter, library: library.id }),
			);
			if (g !== generation) return;
			// LMS can list an album twice across pages (random order, a rescan meanwhile).
			const seen = new Set(albums.map((a) => a.id));
			const fresh = (r.albums_loop ?? []).filter((a) => typeof a?.id === 'number' && !seen.has(a.id) && seen.add(a.id));
			albums = [...albums, ...fresh];
			total = paged ? (r.count ?? albums.length) : albums.length;
			// Nothing new on a page: stop rather than ask for it again and again.
			if (!fresh.length) total = albums.length;
		} catch (e) {
			if (g === generation) error = (e as Error).message;
		} finally {
			if (g === generation) loading = false;
		}
	}

	$effect(() => {
		// Reload whenever what is listed changes.
		void [sort, JSON.stringify(filter), version, library.id, app.libraryVersion];
		untrack(() => {
			generation++;
			albums = [];
			total = null;
			error = '';
			loading = false;
			void more();
		});
	});
</script>

{#if sorts?.length || total !== null}
	<div class="bar">
		{#if total !== null}<p class="muted count">{total.toLocaleString('en-GB')} {total === 1 ? 'album' : 'albums'}</p>{/if}
		{#if sorts?.length}
			<label class="sort">
				<span class="muted">Sort</span>
				<select bind:value={sort}>
					{#each sorts as v (v)}<option value={v}>{SORT_LABELS[v]}</option>{/each}
				</select>
			</label>
		{/if}
	</div>
{/if}
{#if error}<p class="error">{error}</p>{/if}
{#if total === 0 && !error}
	<p class="empty muted">{empty}</p>
{:else}
	<AlbumGrid {albums} {showArtist} more={paged && (total === null || albums.length < total) ? more : null} />
{/if}

<style>
	.bar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		flex-wrap: wrap;
		gap: 0.5rem 1rem;
		margin: 0.25rem 0 1.25rem;
	}
	.count {
		margin: 0;
	}
	.sort {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		margin-left: auto;
	}
	select {
		padding: 0.4em 0.6em;
		border-radius: var(--radius-sm);
		border: 1px solid var(--line);
		background: var(--bg-raised);
	}
	.empty {
		padding: 2rem 0;
	}
</style>
