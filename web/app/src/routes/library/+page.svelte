<script lang="ts">
	import Icon from '$lib/components/Icon.svelte';
	const SECTIONS = [
		{ href: '#/albums', icon: 'album', label: 'Albums' },
		{ href: '#/artists', icon: 'artist', label: 'Artists' },
		{ href: '#/playlists', icon: 'queue', label: 'Playlists' },
		{ href: '#/favourites', icon: 'heart', label: 'Favourites' },
		{ href: '#/apps', icon: 'radio', label: 'Apps & Radio' },
		{ href: '#/downloads', icon: 'download', label: 'Downloads' },
	];
</script>

<section class="page">
	<h1>Library</h1>
	<ul>
		{#each SECTIONS as s (s.href)}
			<li><a href={s.href}><span class="glyph"><Icon name={s.icon} size={22} /></span>{s.label}<span class="chev" aria-hidden="true">›</span></a></li>
		{/each}
	</ul>
</section>

<style>
	ul {
		list-style: none;
		padding: 0;
		margin: 1rem 0 0;
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
	.glyph {
		display: grid;
		place-items: center;
		width: 40px;
		height: 40px;
		border-radius: var(--radius-sm);
		background: var(--accent-soft);
		color: var(--accent);
	}
	.chev {
		margin-left: auto;
		color: var(--fg-muted);
		font-size: 1.4rem;
	}
</style>
