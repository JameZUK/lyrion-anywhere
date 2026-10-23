<script lang="ts">
	// Albums as a list (Library's list view): cover, title, artist and year, a play button and
	// a menu. Swipe a row right to play it next or left to add it to the queue; on a focused
	// row, N and A do the same, and the menu offers both.
	import { app } from '#lib/state/app.svelte';
	import type { Album } from '#lib/lms/types';
	import { coverPath, link } from '#lib/lms/commands';
	import { textOf } from '#lib/lms/format';
	import { rowShortcut, swipeRow } from '#lib/gestures/swipe';
	import Art from './Art.svelte';
	import Icon from './Icon.svelte';
	import RowMenu from './RowMenu.svelte';

	let {
		albums,
		more = null,
		offset = 0,
		showArtist = true,
	}: { albums: Album[]; more?: (() => void) | null; offset?: number; showArtist?: boolean } = $props();

	let sentinel = $state<HTMLDivElement>();
	$effect(() => {
		if (!sentinel || !more || typeof IntersectionObserver === 'undefined') return;
		const load = more;
		const io = new IntersectionObserver((e) => e.some((x) => x.isIntersecting) && load(), { rootMargin: '600px' });
		io.observe(sentinel);
		return () => io.disconnect();
	});

	const next = (a: Album) => app.canQueue && void app.queueUp({ album: a.id }, 'next');
	const add = (a: Album) => app.canQueue && void app.queueUp({ album: a.id }, 'add');
</script>

<p class="sr-only">On an album, press N to play it next or A to add it to the queue.</p>
<ul class="list">
	{#each albums as a, i (a.id)}
		{@const name = textOf(a.album) || 'Album'}
		<li data-pos={offset + i}>
			<div class="row" use:swipeRow={{ onright: () => next(a), onleft: () => add(a) }}>
				<a class="main" href={link('album', a.id) ?? '#/albums'} onkeydown={(e) => rowShortcut(e, { next: () => next(a), add: () => add(a) })} aria-keyshortcuts="N A">
					<span class="thumb"><Art path={coverPath(a.artwork_track_id, 96)} alt="" label={name} /></span>
					<span class="text">
						<span class="name ellipsis">{name}</span>
						<span class="sub ellipsis muted">{[showArtist ? textOf(a.artist) : '', textOf(a.year)].filter(Boolean).join(' · ')}</span>
					</span>
				</a>
				<button class="icon-btn" aria-label={`Play ${name}`} onclick={() => app.queueUp({ album: a.id }, 'play')} disabled={!app.canQueue}><Icon name="play" size={18} /></button>
				<RowMenu
					label={`More options for ${name}`}
					actions={[
						{ label: 'Play next', icon: 'playnext', run: () => next(a), shortcut: 'N', disabled: !app.canQueue },
						{ label: 'Add to queue', icon: 'plus', run: () => add(a), shortcut: 'A', disabled: !app.canQueue },
					]}
				/>
			</div>
		</li>
	{/each}
</ul>
{#if more}<div bind:this={sentinel} class="sentinel" aria-hidden="true"></div>{/if}

<style>
	.list {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 0.15rem;
	}
	li {
		border-radius: var(--radius-sm);
		overflow: hidden;
		background: var(--bg-sunken);
	}
	.row {
		display: flex;
		align-items: center;
		gap: 0.25rem;
		background: var(--bg);
		transition: transform 0.15s ease;
	}
	.row:global([data-swiping]) {
		transition: none;
	}
	.row:hover {
		background: var(--bg-sunken);
	}
	.main {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 0.8rem;
		padding: 0.4rem 0.5rem;
		min-height: 44px;
	}
	.thumb {
		width: 48px;
		flex: none;
	}
	.text {
		display: grid;
		min-width: 0;
	}
	.name {
		font-weight: 600;
	}
	.sub {
		font-size: 0.85rem;
	}
	.sentinel {
		height: 1px;
	}
	@media (prefers-reduced-motion: reduce) {
		.row {
			transition: none;
		}
	}
</style>
