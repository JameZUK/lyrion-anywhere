<script lang="ts">
	import { api, app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { textOf } from '$lib/lms/format';
	import type { SearchResult } from '$lib/lms/types';
	import Icon from '$lib/components/Icon.svelte';

	let term = $state('');
	let result = $state<SearchResult | null>(null);
	let busy = $state(false);
	let error = $state('');
	let timer: ReturnType<typeof setTimeout> | undefined;
	let seq = 0;

	$effect(() => {
		const q = term.trim();
		clearTimeout(timer);
		if (q.length < 2) {
			result = null;
			return;
		}
		timer = setTimeout(async () => {
			const mine = ++seq;
			busy = true;
			try {
				const r = await api.rpc<SearchResult>('', cmd.search(q, 30));
				if (mine === seq) {
					result = r;
					error = '';
				}
			} catch (e) {
				if (mine === seq) error = (e as Error).message;
			} finally {
				if (mine === seq) busy = false;
			}
		}, 250);
	});

	const empty = $derived(
		result && !result.contributors_loop?.length && !result.albums_loop?.length && !result.tracks_loop?.length,
	);
</script>

<section class="page">
	<h1>Search</h1>
	<label class="box">
		<Icon name="search" />
		<span class="sr-only">Search the library</span>
		<!-- svelte-ignore a11y_autofocus -->
		<input type="search" bind:value={term} placeholder="Artists, albums and tracks" autocomplete="off" spellcheck="false" autofocus />
		{#if busy}<span class="spin" aria-label="Searching"></span>{/if}
	</label>
	{#if error}<p class="error">{error}</p>{/if}
	{#if empty}<p class="muted">Nothing found for “{term.trim()}”.</p>{/if}

	{#if result?.contributors_loop?.length}
		<h2>Artists</h2>
		<ul class="list">
			{#each result.contributors_loop as a (a.contributor_id)}
				{@const href = cmd.link('artist', a.contributor_id)}
				{#if href}<li><a {href}><Icon name="artist" size={18} />{textOf(a.contributor)}</a></li>{/if}
			{/each}
		</ul>
	{/if}
	{#if result?.albums_loop?.length}
		<h2>Albums</h2>
		<ul class="list">
			{#each result.albums_loop as a (a.album_id)}
				{@const href = cmd.link('album', a.album_id)}
				{#if href}<li><a {href}><Icon name="album" size={18} />{textOf(a.album)}</a></li>{/if}
			{/each}
		</ul>
	{/if}
	{#if result?.tracks_loop?.length}
		<h2>Tracks</h2>
		<ul class="list">
			{#each result.tracks_loop as t (t.track_id)}
				{#if t.track_id && cmd.link('album', t.track_id)}
					{@const id = Number(t.track_id)}
					<li class="track">
						<button onclick={() => app.queueUp({ track: id }, 'play')}><Icon name="play" size={16} />{textOf(t.track)}</button>
						<button class="icon-btn" aria-label={`Play ${textOf(t.track)} next`} onclick={() => app.queueUp({ track: id }, 'next')}><Icon name="playnext" size={18} /></button>
						<button class="icon-btn" aria-label={`Add ${textOf(t.track)} to the queue`} onclick={() => app.queueUp({ track: id }, 'add')}><Icon name="plus" size={18} /></button>
					</li>
				{/if}
			{/each}
		</ul>
	{/if}
</section>

<style>
	.box {
		display: flex;
		align-items: center;
		gap: 0.6rem;
		max-width: 620px;
		padding: 0.7rem 1rem;
		border: 1px solid var(--line);
		border-radius: 999px;
		background: var(--bg-raised);
		color: var(--fg-muted);
		margin-bottom: 1.5rem;
	}
	.box:focus-within {
		border-color: var(--accent);
	}
	input {
		flex: 1;
		border: 0;
		background: transparent;
		outline: none;
		font-size: 1.05rem;
		color: var(--fg);
	}
	h2 {
		margin-top: 1.5rem;
	}
	.list {
		list-style: none;
		padding: 0;
		margin: 0;
		display: grid;
		gap: 0.1rem;
	}
	.list a,
	.track > button:first-child {
		display: flex;
		align-items: center;
		gap: 0.7rem;
		padding: 0.5rem 0.6rem;
		border-radius: var(--radius-sm);
		border: 0;
		background: transparent;
		text-align: left;
		flex: 1;
		min-width: 0;
	}
	.list a:hover,
	.track > button:first-child:hover {
		background: var(--bg-sunken);
	}
	.track {
		display: flex;
		align-items: center;
	}
	.spin {
		width: 16px;
		height: 16px;
		border: 2px solid var(--line);
		border-top-color: var(--accent);
		border-radius: 50%;
		animation: spin 0.8s linear infinite;
	}
	@keyframes spin {
		to {
			transform: rotate(360deg);
		}
	}
</style>
