<script lang="ts">
	// A row of albums on Home, scrolled sideways.
	import type { Album } from '#lib/lms/types';
	import { coverPath, link } from '#lib/lms/commands';
	import { textOf } from '#lib/lms/format';
	import Art from './Art.svelte';

	let { albums, label }: { albums: Album[]; label: string } = $props();
</script>

<ul class="row" aria-label={label}>
	{#each albums as a (a.id)}
		<li>
			<a href={link('album', a.id) ?? '#/albums'} class="tile">
				<Art path={coverPath(a.artwork_track_id, 300)} alt="" label={textOf(a.album)} />
				<span class="name ellipsis">{textOf(a.album)}</span>
				<span class="by ellipsis muted">{textOf(a.artist)}</span>
			</a>
		</li>
	{/each}
</ul>

<style>
	.row {
		list-style: none;
		margin: 0;
		padding: 0.25rem 0 0.5rem;
		display: grid;
		grid-auto-flow: column;
		grid-auto-columns: minmax(132px, 156px);
		gap: 1rem;
		overflow-x: auto;
		scroll-snap-type: x proximity;
		overscroll-behavior-x: contain;
	}
	li {
		scroll-snap-align: start;
		min-width: 0;
	}
	.tile {
		display: grid;
		gap: 0.15rem;
		border-radius: var(--radius-sm);
	}
	.tile :global(.art) {
		box-shadow: var(--shadow);
		margin-bottom: 0.4rem;
	}
	.name {
		font-weight: 600;
		font-size: 0.95rem;
	}
	.by {
		font-size: 0.82rem;
	}
</style>
