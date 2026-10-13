<script lang="ts">
	// One shelf on Home, by type (see lib/home/layout.ts).
	import { untrack } from 'svelte';
	import { api, app } from '$lib/state/app.svelte';
	import { me } from '$lib/state/me';
	import * as cmd from '$lib/lms/commands';
	import { textOf } from '$lib/lms/format';
	import type { Album, AlbumsResult, FavouritesResult, MenuItem, Playlist, PlaylistsResult } from '$lib/lms/types';
	import type { HomeShelf, PinnedMenu, SmartFilter, Station } from '$lib/api/me';
	import { startFilter } from '$lib/filters/run';
	import Icon from './Icon.svelte';
	import type { SBMenu } from '$lib/lms/slimbrowse';
	import { SHELF_LABELS } from '$lib/home/layout';
	import AlbumShelf from './AlbumShelf.svelte';
	import StationCards from './StationCards.svelte';
	import MenuView from './MenuView.svelte';

	let {
		shelf,
		stations,
		menus,
		filters = [],
		onstationremoved,
	}: { shelf: HomeShelf; stations: Station[]; menus: PinnedMenu[]; filters?: SmartFilter[]; onstationremoved?: (id: number) => void } = $props();

	const COUNT = 20;
	let albums = $state<Album[] | null>(null);
	let menu = $state<SBMenu | null>(null);
	let genreName = $state('');
	let favourites = $state<MenuItem[] | null>(null);
	let playlists = $state<Playlist[] | null>(null);
	let error = $state('');

	const station = $derived(shelf.type === 'station' ? stations.find((s) => s.id === shelf.id) : undefined);
	const pinned = $derived(shelf.type === 'menu' ? menus.find((m) => m.id === shelf.id) : undefined);
	const smart = $derived(shelf.type === 'filter' ? filters.find((f) => f.id === shelf.id) : undefined);
	const title = $derived(
		shelf.type === 'station'
			? (station?.name ?? 'Station')
			: shelf.type === 'menu'
				? (pinned?.name ?? 'Pinned menu')
				: shelf.type === 'genre'
					? genreName || 'Genre'
					: shelf.type === 'filter'
						? (smart?.name ?? 'Smart filter')
						: SHELF_LABELS[shelf.type],
	);
	let playMessage = $state('');
	async function playSmart(shuffle: boolean) {
		if (shelf.type !== 'filter' || shelf.id === undefined) return;
		playMessage = '';
		playMessage = await startFilter(shelf.id, shuffle);
	}
	const EMPTY: Partial<Record<HomeShelf['type'], string>> = {
		recent: 'Nothing played yet. What you play shows here.',
		most: 'Play some music and your favourites show here.',
		new: 'Nothing has been added to the library yet.',
		random: 'No albums in the library yet.',
		genre: 'No albums in this genre.',
		filter: 'No albums match this filter.',
	};

	async function byIds(ids: number[]): Promise<Album[]> {
		if (!ids.length) return [];
		const r = await api.rpc<AlbumsResult>('', cmd.albumsByIds(ids));
		return cmd.inOrder(r.albums_loop ?? [], ids);
	}

	async function load(s: HomeShelf, player: string) {
		error = '';
		try {
			switch (s.type) {
				case 'recent':
				case 'most':
				case 'on_this_day':
					albums = await byIds(await me.shelf(s.type, COUNT));
					break;
				case 'new':
				case 'random':
					albums = (await api.rpc<AlbumsResult>('', cmd.albums(0, COUNT, s.type))).albums_loop ?? [];
					break;
				case 'genre': {
					const [list, g] = await Promise.all([
						api.rpc<AlbumsResult>('', cmd.albums(0, COUNT, 'album', undefined, { genre: s.id })),
						api.rpc<{ genres_loop?: { genre?: string }[] }>('', cmd.genreById(s.id!)),
					]);
					albums = list.albums_loop ?? [];
					genreName = textOf(g.genres_loop?.[0]?.genre);
					break;
				}
				case 'filter':
					albums = await byIds((await me.filterAlbums(s.id!, 0, COUNT)).albums);
					break;
				case 'favourites':
					favourites = ((await api.rpc<FavouritesResult>('', cmd.favourites(0, COUNT))).loop_loop ?? []).filter((f) => typeof f.id === 'string');
					break;
				case 'playlists':
					playlists = ((await api.rpc<PlaylistsResult>('', cmd.playlists(0, COUNT))).playlists_loop ?? []).filter((p) => Number.isSafeInteger(p.id) && p.id > 0);
					break;
				case 'menu':
					if (player) menu = await me.openMenu<SBMenu>(s.id!, player);
					break;
			}
		} catch (e) {
			error = (e as Error).message;
		}
	}

	$effect(() => {
		const s = shelf;
		const player = s.type === 'menu' ? app.playerId : '';
		void app.libraryVersion;
		untrack(() => void load(s, player));
	});

	// "On this day" shows only when there is something.
	const hide = $derived(shelf.type === 'on_this_day' && albums !== null && albums.length === 0 && !error);
</script>

{#if !hide}
	<section class="shelf" aria-labelledby={`shelf-${shelf.type}-${shelf.id ?? 0}`}>
		{#if shelf.type === 'filter'}
			<div class="head">
				<h2 id={`shelf-${shelf.type}-${shelf.id ?? 0}`}><a href={`#/filter/${shelf.id}`}>{title}</a></h2>
				{#if albums?.length}
					<button class="icon-btn" aria-label={`Play ${title}`} onclick={() => playSmart(false)} data-shelf-filter-play><Icon name="play" size={18} /></button>
					<button class="icon-btn" aria-label={`Shuffle ${title}`} onclick={() => playSmart(true)} data-shelf-filter-shuffle><Icon name="shuffle" size={18} /></button>
				{/if}
			</div>
			{#if playMessage}<p class="error" role="alert">{playMessage}</p>{/if}
		{:else}
			<h2 id={`shelf-${shelf.type}-${shelf.id ?? 0}`}>{title}</h2>
		{/if}
		{#if error}
			<p class="error">{error}</p>
		{:else if shelf.type === 'stations'}
			<StationCards {stations} removable onremoved={onstationremoved} />
			<a class="link small" href="#/stations">All stations</a>
		{:else if shelf.type === 'station'}
			{#if station}<StationCards stations={[station]} />{:else}<p class="muted">This station has been deleted.</p>{/if}
		{:else if shelf.type === 'menu'}
			{#if !app.playerId}
				<p class="muted">Choose a player first: menus browse and play for a player.</p>
			{:else if menu}
				<MenuView initial={menu} title={title} compact />
			{:else}
				<p class="muted">Loading…</p>
			{/if}
		{:else if shelf.type === 'favourites'}
			{#if favourites === null}
				<p class="muted">Loading…</p>
			{:else if favourites.length}
				<ul class="chips">
					{#each favourites as f (f.id)}
						<li>
							{#if Number(f.isaudio) === 1}
								<button class="chip" onclick={() => app.send(cmd.playFavourite(f.id!, 'play'))} aria-label={`Play ${textOf(f.name)}`}>{textOf(f.name)}</button>
							{:else}
								<a class="chip" href="#/favourites">{textOf(f.name)}</a>
							{/if}
						</li>
					{/each}
				</ul>
			{:else}
				<p class="muted">No favourites yet.</p>
			{/if}
		{:else if shelf.type === 'playlists'}
			{#if playlists === null}
				<p class="muted">Loading…</p>
			{:else if playlists.length}
				<ul class="chips">
					{#each playlists as p (p.id)}
						<li><a class="chip" href={cmd.link('playlist', p.id) ?? '#/playlists'}>{textOf(p.playlist)}</a></li>
					{/each}
				</ul>
			{:else}
				<p class="muted">No saved playlists yet.</p>
			{/if}
		{:else if albums === null}
			<p class="muted">Loading…</p>
		{:else if albums.length}
			<AlbumShelf {albums} label={title} />
		{:else}
			<p class="muted">{EMPTY[shelf.type] ?? 'Nothing here yet.'}</p>
		{/if}
	</section>
{/if}

<style>
	.shelf {
		margin: 0 0 1.5rem;
		min-width: 0;
	}
	h2 {
		font-size: 1.2rem;
		margin: 0 0 0.5rem;
	}
	.head {
		display: flex;
		align-items: center;
		gap: 0.25rem;
	}
	.head h2 {
		flex: 1;
		min-width: 0;
		margin: 0;
	}
	.head .icon-btn {
		width: 44px;
		height: 44px;
	}
	.small {
		font-size: 0.9rem;
	}
	.chips {
		list-style: none;
		margin: 0;
		padding: 0.25rem 0 0.5rem;
		display: flex;
		gap: 0.5rem;
		overflow-x: auto;
	}
	.chip {
		display: inline-block;
		white-space: nowrap;
		padding: 0.5rem 0.9rem;
		border-radius: 999px;
		border: 1px solid var(--line);
		background: var(--bg-raised);
		color: inherit;
		font-weight: 600;
	}
</style>
