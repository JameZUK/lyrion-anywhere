<script lang="ts">
	import Icon from '$lib/components/Icon.svelte';
	import { library } from '$lib/state/library.svelte';

	// LMS's "My Music", then the app's other collections.
	const BROWSE = [
		{ href: '#/albums', icon: 'album', label: 'Albums' },
		{ href: '#/artists', icon: 'artist', label: 'Artists' },
		{ href: '#/genres', icon: 'genre', label: 'Genres' },
		{ href: '#/years', icon: 'calendar', label: 'Years' },
		{ href: '#/compilations', icon: 'stack', label: 'Compilations' },
		{ href: '#/random', icon: 'shuffle', label: 'Random Albums' },
		{ href: '#/folder', icon: 'folder', label: 'Music Folder' },
	];
	const MORE = [
		{ href: '#/playlists', icon: 'queue', label: 'Playlists' },
		{ href: '#/favourites', icon: 'heart', label: 'Favourites' },
		{ href: '#/apps', icon: 'radio', label: 'Apps & Radio' },
		{ href: '#/downloads', icon: 'download', label: 'Downloads' },
	];
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
	<a class="hero" href="#/new">
		<span class="glyph"><Icon name="new" size={26} /></span>
		<span class="words"><strong>New Music</strong><span class="muted">Recently added albums</span></span>
		<span class="chev" aria-hidden="true">›</span>
	</a>
	<ul>
		{#each BROWSE as s (s.href)}
			<li><a href={s.href}><span class="glyph"><Icon name={s.icon} size={22} /></span>{s.label}<span class="chev" aria-hidden="true">›</span></a></li>
		{/each}
	</ul>
	<h2 class="muted">More</h2>
	<ul>
		{#each MORE as s (s.href)}
			<li><a href={s.href}><span class="glyph"><Icon name={s.icon} size={22} /></span>{s.label}<span class="chev" aria-hidden="true">›</span></a></li>
		{/each}
	</ul>
</section>

<style>
	ul {
		list-style: none;
		padding: 0;
		margin: 0.75rem 0 0;
		display: grid;
		gap: 0.4rem;
		max-width: 560px;
	}
	a {
		display: flex;
		align-items: center;
		gap: 0.9rem;
		padding: 0.8rem 0.9rem;
		border-radius: var(--radius);
		background: var(--bg-raised);
		border: 1px solid var(--line);
		font-weight: 600;
		font-size: 1.05rem;
	}
	.hero {
		max-width: 560px;
		box-sizing: border-box;
		margin-top: 1rem;
		padding: 1rem 0.9rem;
		background: var(--accent-soft);
		border-color: transparent;
		color: var(--accent-soft-fg);
	}
	.hero .glyph {
		width: 48px;
		height: 48px;
		background: var(--accent);
		color: var(--bg);
	}
	.words {
		display: grid;
		gap: 0.1rem;
	}
	.words .muted {
		font-weight: 500;
		font-size: 0.9rem;
		color: inherit;
		opacity: 0.8;
	}
	.glyph {
		display: grid;
		place-items: center;
		width: 40px;
		height: 40px;
		border-radius: var(--radius-sm);
		background: var(--accent-soft);
		color: var(--accent);
		flex: none;
	}
	.chev {
		margin-left: auto;
		color: var(--fg-muted);
		font-size: 1.4rem;
	}
	.hero .chev {
		color: inherit;
	}
	h2 {
		font-size: 0.85rem;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		margin: 1.5rem 0 0;
	}
	.picker {
		max-width: 560px;
		margin-top: 0.75rem;
	}
</style>
