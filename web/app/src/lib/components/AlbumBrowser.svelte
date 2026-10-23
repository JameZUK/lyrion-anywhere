<script lang="ts" module>
	import type { AlbumSort } from '#lib/lms/commands';
	/**
	 * An album order: one of LMS's, or `played`, the user's own recently played albums (from
	 * their listening history on the gateway; LMS's `recentlyplayed` is everyone's).
	 */
	export type BrowseSort = AlbumSort | 'played';
	export const SORT_LABELS: Record<BrowseSort, string> = {
		played: 'Recently played',
		album: 'A–Z',
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
	// Optionally a grid/list toggle and, sorted A–Z, a fast scroller built from LMS's index.
	import { tick, untrack } from 'svelte';
	import { api, app } from '#lib/state/app.svelte';
	import { library } from '#lib/state/library.svelte';
	import * as cmd from '#lib/lms/commands';
	import type { IndexEntry } from '#lib/lms/azindex';
	import { WindowPager } from '#lib/lms/pager.svelte';
	import type { Album, AlbumsResult } from '#lib/lms/types';
	import type { AlbumView } from '#lib/library/prefs';
	import { me } from '#lib/state/me';
	import AlbumGrid from './AlbumGrid.svelte';
	import AlbumList from './AlbumList.svelte';
	import AzScroller from './AzScroller.svelte';
	import Icon from './Icon.svelte';

	let {
		filter = {},
		sort = $bindable('album'),
		sorts = ALBUM_SORTS,
		paged = true,
		pageSize = 120,
		version = 0,
		showArtist = true,
		empty = 'No albums here.',
		view = $bindable('grid'),
		views = false,
		az = false,
	}: {
		filter?: cmd.Filter;
		sort?: BrowseSort;
		/** Orders to offer; none for a fixed order. */
		sorts?: BrowseSort[] | readonly BrowseSort[] | null;
		/** Page as the list scrolls; otherwise one batch of `pageSize`. */
		paged?: boolean;
		pageSize?: number;
		/** Bump to load the list again (e.g. "Shuffle again"). */
		version?: number;
		showArtist?: boolean;
		empty?: string;
		view?: AlbumView;
		/** Offer the grid/list toggle. */
		views?: boolean;
		/** Offer the A–Z scroller when sorted by title. */
		az?: boolean;
	} = $props();

	let root = $state<HTMLDivElement>();
	const key = (a: Album) => (typeof a?.id === 'number' ? a.id : null);
	/** The user's recently played albums: at most this many (newest first). */
	const PLAYED_MAX = 50;
	const query = (s: BrowseSort, f: cmd.Filter, lib: string | null, wantIndex: boolean) => async (from: number, n: number, first: boolean) => {
		if (s === 'played') {
			const all = await me.shelf('recent', PLAYED_MAX);
			const ids = all.slice(from, from + n);
			const rows = ids.length ? cmd.inOrder((await api.rpc<AlbumsResult>('', cmd.albumsByIds(ids)))?.albums_loop ?? [], ids) : [];
			return { rows, count: paged ? all.length : undefined };
		}
		const r = await api.rpc<AlbumsResult & { indexList?: unknown }>('', cmd.albums(from, n, s, undefined, { ...f, library: lib }, wantIndex && first));
		return { rows: r?.albums_loop ?? [], count: paged ? r?.count : undefined, ...(wantIndex && first ? { indexList: r?.indexList } : {}) };
	};
	const list = new WindowPager<Album>(query('album', {}, null, false), key, untrack(() => pageSize), untrack(() => !paged));

	$effect(() => {
		// Reload whenever what is listed changes.
		const s = sort;
		const f = filter;
		void [JSON.stringify(f), version, library.id, app.libraryVersion];
		const lib = library.id;
		untrack(() => void list.reset(query(s, f, lib, az && s === 'album')));
	});

	const index = $derived(az && sort === 'album' ? list.index : []);
	const more = $derived(paged && !list.done ? () => void list.more() : null);

	async function jump(e: IndexEntry) {
		await list.jumpTo(e.offset);
		await tick();
		const target = root?.querySelector<HTMLElement>(`[data-pos="${e.offset}"] a`);
		target?.scrollIntoView?.({ block: 'start' });
		target?.focus({ preventScroll: true });
	}
</script>

<div class="browser" bind:this={root}>
	{#if sorts?.length || views || list.total !== null}
		<div class="bar">
			{#if list.total !== null}<p class="muted count">{list.total.toLocaleString('en-GB')} {list.total === 1 ? 'album' : 'albums'}</p>{/if}
			<div class="tools">
				{#if sorts?.length}
					<label class="sort">
						<span class="muted">Sort</span>
						<select bind:value={sort}>
							{#each sorts as v (v)}<option value={v}>{SORT_LABELS[v]}</option>{/each}
						</select>
					</label>
				{/if}
				{#if views}
					<div class="views" role="group" aria-label="Show albums as">
						<button type="button" class="icon-btn" class:on={view === 'grid'} aria-pressed={view === 'grid'} aria-label="Grid" onclick={() => (view = 'grid')} data-view="grid"><Icon name="shelves" size={18} /></button>
						<button type="button" class="icon-btn" class:on={view === 'list'} aria-pressed={view === 'list'} aria-label="List" onclick={() => (view = 'list')} data-view="list"><Icon name="queue" size={18} /></button>
					</div>
				{/if}
			</div>
		</div>
	{/if}
	{#if list.error}<p class="error">{list.error}</p>{/if}
	{#if list.total === 0 && !list.error}
		<p class="empty muted">{empty}</p>
	{:else}
		<AzScroller {index} onjump={jump} noun="albums" />
		{#if list.base > 0}
			<button class="btn earlier" onclick={() => list.earlier()} data-earlier><Icon name="up" size={18} />Show earlier albums</button>
		{/if}
		{#if view === 'list'}
			<AlbumList albums={list.items} offset={list.base} {showArtist} {more} />
		{:else}
			<AlbumGrid albums={list.items} offset={list.base} {showArtist} {more} />
		{/if}
	{/if}
</div>

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
	.tools {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		margin-left: auto;
	}
	.sort {
		display: flex;
		align-items: center;
		gap: 0.5rem;
	}
	select {
		min-height: 44px;
		padding: 0.4em 0.6em;
		border-radius: var(--radius-sm);
		border: 1px solid var(--line);
		background: var(--bg-raised);
	}
	.views {
		display: flex;
		gap: 0.2rem;
	}
	.views .on {
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
	}
	.earlier {
		margin: 0 0 1rem;
	}
	.empty {
		padding: 2rem 0;
	}
</style>
