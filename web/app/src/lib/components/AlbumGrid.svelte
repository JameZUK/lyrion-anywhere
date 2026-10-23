<script lang="ts">
	import type { Album } from '#lib/lms/types';
	import { coverPath, link } from '#lib/lms/commands';
	import { textOf } from '#lib/lms/format';
	import Art from './Art.svelte';

	let {
		albums,
		more = null,
		showArtist = true,
		offset = 0,
	}: { albums: Album[]; more?: (() => void) | null; showArtist?: boolean; /** Where the first album sits in the whole list. */ offset?: number } = $props();

	let sentinel = $state<HTMLDivElement>();
	$effect(() => {
		if (!sentinel || !more) return;
		const load = more;
		const io = new IntersectionObserver((e) => e.some((x) => x.isIntersecting) && load(), { rootMargin: '600px' });
		io.observe(sentinel);
		return () => io.disconnect();
	});
</script>

<ul class="grid">
	{#each albums as a, i (a.id)}
		<li data-pos={offset + i}>
			<a href={link('album', a.id) ?? '#/albums'} class="tile">
				<Art path={coverPath(a.artwork_track_id, 300)} alt="" label={textOf(a.album)} />
				<span class="name ellipsis">{textOf(a.album)}</span>
				{#if showArtist}<span class="by ellipsis muted">{textOf(a.artist)}</span>{/if}
				{#if !showArtist && a.year}<span class="by muted">{textOf(a.year)}</span>{/if}
			</a>
		</li>
	{/each}
</ul>
{#if more}<div bind:this={sentinel} class="sentinel" aria-hidden="true"></div>{/if}

<style>
	.grid {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(min(42vw, 168px), 1fr));
		gap: 1.25rem 1rem;
	}
	.tile {
		display: grid;
		gap: 0.15rem;
		border-radius: var(--radius-sm);
	}
	.tile :global(.art) {
		box-shadow: var(--shadow);
		transition: transform 0.15s ease-out;
		margin-bottom: 0.45rem;
	}
	.tile:hover :global(.art) {
		transform: translateY(-2px);
	}
	.name {
		font-weight: 600;
	}
	.by {
		font-size: 0.85rem;
	}
	.sentinel {
		height: 1px;
	}
</style>
