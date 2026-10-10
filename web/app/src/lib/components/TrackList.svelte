<script lang="ts">
	import type { Track } from '$lib/lms/types';
	import { duration, textOf } from '$lib/lms/format';
	import Icon from './Icon.svelte';

	let {
		tracks,
		onplay,
		onnext,
		onadd,
		showArtist = false,
	}: {
		tracks: Track[];
		onplay: (t: Track, index: number) => void;
		onnext?: (t: Track) => void;
		onadd?: (t: Track) => void;
		showArtist?: boolean;
	} = $props();

	let open = $state<number | null>(null);
</script>

<ol class="tracks">
	{#each tracks as t, i (t.id ?? i)}
		<li class="row">
			<button class="main" onclick={() => onplay(t, i)} aria-label={`Play ${textOf(t.title)}`}>
				<span class="num muted">{textOf(t.tracknum) || i + 1}</span>
				<span class="text">
					<span class="title ellipsis">{textOf(t.title)}</span>
					{#if showArtist && t.artist}<span class="artist ellipsis muted">{textOf(t.artist)}</span>{/if}
				</span>
				<span class="dur muted">{duration(t.duration)}</span>
			</button>
			{#if onnext || onadd}
				<div class="more">
					<button class="icon-btn" aria-label="More options" aria-expanded={open === i} onclick={() => (open = open === i ? null : i)}>
						<Icon name="more" size={20} />
					</button>
					{#if open === i}
						<div class="menu" role="menu">
							{#if onnext}<button role="menuitem" onclick={() => { onnext(t); open = null; }}><Icon name="playnext" size={18} />Play next</button>{/if}
							{#if onadd}<button role="menuitem" onclick={() => { onadd(t); open = null; }}><Icon name="plus" size={18} />Add to queue</button>{/if}
						</div>
					{/if}
				</div>
			{/if}
		</li>
	{/each}
</ol>

<style>
	.tracks {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	.row {
		display: flex;
		align-items: center;
		border-radius: var(--radius-sm);
	}
	.row:hover {
		background: var(--bg-sunken);
	}
	.main {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 0.9rem;
		padding: 0.6rem 0.6rem;
		border: 0;
		background: transparent;
		text-align: left;
	}
	.num {
		width: 2ch;
		text-align: right;
		font-variant-numeric: tabular-nums;
	}
	.text {
		flex: 1;
		min-width: 0;
		display: grid;
	}
	.artist {
		font-size: 0.85rem;
	}
	.dur {
		font-variant-numeric: tabular-nums;
		font-size: 0.9rem;
	}
	.more {
		position: relative;
	}
	.menu {
		position: absolute;
		right: 0;
		top: 100%;
		z-index: 5;
		display: grid;
		min-width: 180px;
		padding: 0.35rem;
		background: var(--bg-raised);
		border: 1px solid var(--line);
		border-radius: var(--radius-sm);
		box-shadow: var(--shadow);
	}
	.menu button {
		display: flex;
		align-items: center;
		gap: 0.6rem;
		padding: 0.55rem 0.7rem;
		border: 0;
		background: transparent;
		border-radius: 6px;
		text-align: left;
	}
	.menu button:hover {
		background: var(--bg-sunken);
	}
</style>
