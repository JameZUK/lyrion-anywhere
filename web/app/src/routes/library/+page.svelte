<script lang="ts">
	// Library (Wave 2 §3.2): type chips (Albums, Artists, Playlists, Favourites), album order,
	// grid or list, and the A–Z fast scroller; the choice is remembered on this device. LMS's
	// other ways in (genres, years, the music folder…) are links above.
	import Icon from '$lib/components/Icon.svelte';
	import Chips from '$lib/components/Chips.svelte';
	import AlbumBrowser from '$lib/components/AlbumBrowser.svelte';
	import ArtistBrowser from '$lib/components/ArtistBrowser.svelte';
	import PlaylistsView from '$lib/components/PlaylistsView.svelte';
	import FavouritesView from '$lib/components/FavouritesView.svelte';
	import { library } from '$lib/state/library.svelte';
	import { CHIPS, CHIP_LABELS, LIBRARY_SORTS, loadLibraryPrefs, saveLibraryPrefs, type AlbumView, type LibraryChip } from '$lib/library/prefs';
	import type { AlbumSort } from '$lib/lms/commands';

	const BROWSE = [
		{ href: '#/new', icon: 'new', label: 'New music' },
		{ href: '#/genres', icon: 'genre', label: 'Genres' },
		{ href: '#/years', icon: 'calendar', label: 'Years' },
		{ href: '#/compilations', icon: 'stack', label: 'Compilations' },
		{ href: '#/random', icon: 'shuffle', label: 'Random albums' },
		{ href: '#/folder', icon: 'folder', label: 'Music folder' },
		{ href: '#/apps', icon: 'radio', label: 'Apps & radio' },
		{ href: '#/downloads', icon: 'download', label: 'Downloads' },
	];

	const saved = loadLibraryPrefs();
	let chip = $state<LibraryChip>(saved.chip);
	let sort = $state<AlbumSort>(saved.sort);
	let view = $state<AlbumView>(saved.view);
	const panel = 'library-panel';

	$effect(() => {
		saveLibraryPrefs({ chip, sort, view });
	});
</script>

<section class="page">
	<h1>Library</h1>
	{#if library.available.length}
		<label class="field picker">
			<span>Library</span>
			<select value={library.id ?? ''} onchange={(e) => library.choose(e.currentTarget.value || null)}>
				<option value="">Whole library</option>
				{#each library.available as l (l.id)}<option value={l.id}>{l.name}</option>{/each}
			</select>
		</label>
	{/if}
	<nav class="browse" aria-label="Browse">
		<ul>
			{#each BROWSE as s (s.href)}
				<li><a href={s.href}><Icon name={s.icon} size={18} />{s.label}</a></li>
			{/each}
		</ul>
	</nav>
	<Chips
		items={CHIPS.map((c) => ({ value: c, label: CHIP_LABELS[c] }))}
		value={chip}
		onchange={(c) => (chip = c)}
		label="Show in Library"
		idPrefix="library-chip"
		{panel}
	/>
	<div id={panel} class="panel" role="tabpanel" aria-labelledby={`library-chip-${chip}`} data-library-panel={chip}>
		{#if chip === 'albums'}
			<AlbumBrowser bind:sort bind:view sorts={LIBRARY_SORTS} views az empty="No albums in the library yet." />
		{:else if chip === 'artists'}
			<ArtistBrowser />
		{:else if chip === 'playlists'}
			<PlaylistsView level={2} />
		{:else}
			<FavouritesView level={2} />
		{/if}
	</div>
</section>

<style>
	.browse ul {
		list-style: none;
		padding: 0;
		margin: 0.75rem 0 0.5rem;
		display: flex;
		gap: 0.4rem;
		overflow-x: auto;
		scrollbar-width: none;
	}
	.browse a {
		display: inline-flex;
		align-items: center;
		gap: 0.45rem;
		min-height: 44px;
		box-sizing: border-box;
		padding: 0 0.85rem;
		border-radius: var(--radius-sm);
		background: var(--bg-raised);
		border: 1px solid var(--line);
		font-weight: 600;
		font-size: 0.9rem;
		white-space: nowrap;
		color: var(--fg);
	}
	.browse a :global(svg) {
		color: var(--accent);
	}
	.panel {
		margin-top: 0.75rem;
	}
	.picker {
		max-width: 560px;
		margin-top: 0.75rem;
	}
</style>
