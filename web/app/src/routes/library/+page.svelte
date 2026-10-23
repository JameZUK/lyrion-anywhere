<script lang="ts">
	// Library (Wave 2 §3.2; one model since Wave 3.5): one row of sections (Albums, Artists,
	// Playlists, Favourites, Smart playlists, Genres, Years, Compilations, Folders, Apps & radio,
	// Downloads), each shown here in place. New music and Random albums are orders of Albums
	// ("Recently added", "Random"); the old pages of their own (/albums, /artists, /new,
	// /random) open Library at that section. The virtual library choice is in the header's ⋯.
	// The section, the album order and grid or list are remembered on this device.
	import Chips from '#lib/components/Chips.svelte';
	import AlbumBrowser from '#lib/components/AlbumBrowser.svelte';
	import ArtistBrowser from '#lib/components/ArtistBrowser.svelte';
	import PlaylistsView from '#lib/components/PlaylistsView.svelte';
	import FavouritesView from '#lib/components/FavouritesView.svelte';
	import FiltersView from '#lib/components/FiltersView.svelte';
	import GenresView from '#lib/components/GenresView.svelte';
	import YearsView from '#lib/components/YearsView.svelte';
	import FolderView from '#lib/components/FolderView.svelte';
	import AppsView from '#lib/components/AppsView.svelte';
	import DownloadsView from '#lib/components/DownloadsView.svelte';
	import LibraryNote from '#lib/components/LibraryNote.svelte';
	import Icon from '#lib/components/Icon.svelte';
	import RowMenu, { type RowAction } from '#lib/components/RowMenu.svelte';
	import { library } from '#lib/state/library.svelte';
	import { CHIPS, CHIP_LABELS, LIBRARY_SORTS, loadLibraryPrefs, saveLibraryPrefs, type AlbumView, type LibraryChip } from '#lib/library/prefs';
	import type { BrowseSort } from '#lib/components/AlbumBrowser.svelte';

	const saved = loadLibraryPrefs();
	let chip = $state<LibraryChip>(saved.chip);
	let sort = $state<BrowseSort>(saved.sort);
	let view = $state<AlbumView>(saved.view);
	/** Bumped by "Shuffle again" (the Random order). */
	let shuffled = $state(0);
	const panel = 'library-panel';

	$effect(() => {
		saveLibraryPrefs({ chip, sort, view });
	});

	/** The header's ⋯: which virtual library to show (when LMS has any). */
	const libraryActions = $derived.by((): RowAction[] =>
		library.available.length
			? [
					{ label: 'Whole library', icon: library.id ? 'library' : 'check', run: () => library.choose(null), checked: !library.id, id: 'library-all' },
					...library.available.map((l) => ({ label: l.name, icon: library.id === l.id ? 'check' : 'library', run: () => library.choose(l.id), checked: library.id === l.id })),
				]
			: [],
	);
</script>

<section class="page">
	<header class="head">
		<h1>Library</h1>
		{#if libraryActions.length}
			<RowMenu label="Library options: which library to show" actions={libraryActions} text={library.name || 'Whole library'} icon="library" trigger={{ 'data-library-picker': true }} />
		{/if}
	</header>
	<Chips
		items={CHIPS.map((c) => ({ value: c, label: CHIP_LABELS[c] }))}
		value={chip}
		onchange={(c) => (chip = c)}
		label="Library sections"
		idPrefix="library-chip"
		{panel}
		scroll
	/>
	<div id={panel} class="panel" role="tabpanel" aria-labelledby={`library-chip-${chip}`} data-library-panel={chip}>
		{#if chip === 'albums'}
			<LibraryNote />
			{#key sort === 'random'}
				{#if sort === 'random'}
					<!-- One batch: paging a random order would only repeat albums. -->
					<button class="btn again" onclick={() => shuffled++} data-shuffle-again><Icon name="shuffle" size={18} />Shuffle again</button>
					<AlbumBrowser bind:sort bind:view sorts={LIBRARY_SORTS} views paged={false} pageSize={60} version={shuffled} empty="No albums in the library yet." />
				{:else}
					<AlbumBrowser bind:sort bind:view sorts={LIBRARY_SORTS} views az empty="No albums in the library yet." />
				{/if}
			{/key}
		{:else if chip === 'artists'}
			<LibraryNote />
			<ArtistBrowser />
		{:else if chip === 'playlists'}
			<PlaylistsView level={2} />
		{:else if chip === 'filters'}
			<FiltersView level={2} />
		{:else if chip === 'favourites'}
			<FavouritesView level={2} />
		{:else if chip === 'genres'}
			<GenresView embedded />
		{:else if chip === 'years'}
			<YearsView embedded />
		{:else if chip === 'compilations'}
			<LibraryNote />
			<AlbumBrowser filter={{ compilation: true }} sorts={['album', 'yearalbum', 'new']} showArtist={false} empty="No compilations in the library." />
		{:else if chip === 'folders'}
			<FolderView embedded />
		{:else if chip === 'apps'}
			<AppsView embedded />
		{:else}
			<DownloadsView embedded />
		{/if}
	</div>
</section>

<style>
	.head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 0.5rem 1rem;
		flex-wrap: wrap;
	}
	h1 {
		margin: 0;
	}
	.panel {
		margin-top: 0.5rem;
	}
	.again {
		margin-bottom: 0.5rem;
	}
</style>
