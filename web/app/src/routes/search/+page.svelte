<script lang="ts">
	// Library search. The term and results live in `search` (not the page), so going to a
	// result and back finds them as they were.
	import { app } from '$lib/state/app.svelte';
	import { search } from '$lib/state/search';
	import * as cmd from '$lib/lms/commands';
	import { textOf } from '$lib/lms/format';
	import Icon from '$lib/components/Icon.svelte';

	const result = $derived(search.result);
	const found = $derived(result ? (result.contributors_loop?.length ?? 0) + (result.albums_loop?.length ?? 0) + (result.tracks_loop?.length ?? 0) : 0);
	const empty = $derived(!!result && found === 0);
	/** What a screen reader hears once a search is answered. */
	const said = $derived(search.busy || !result ? '' : empty ? `Nothing found for “${search.shownFor}”.` : `${found} ${found === 1 ? 'result' : 'results'} for “${search.shownFor}”.`);
	// Back from a result: don't pop the keyboard up over the results.
	const focusBox = !search.term;
</script>

<section class="page">
	<h1>Search</h1>
	<label class="box">
		<Icon name="search" />
		<span class="sr-only">Search the library</span>
		<!-- svelte-ignore a11y_autofocus -->
		<input
			type="search"
			value={search.term}
			oninput={(e) => search.set(e.currentTarget.value)}
			placeholder="Artists, albums and tracks"
			autocomplete="off"
			spellcheck="false"
			autofocus={focusBox}
			data-search-input
		/>
		{#if search.busy}<span class="spin" role="img" aria-label="Searching"></span>{/if}
	</label>
	{#if search.error}<p class="error" role="alert">{search.error}</p>{/if}
	<p class="sr-only" role="status">{said}</p>
	{#if empty}<p class="muted" aria-hidden="true">Nothing found for “{search.shownFor}”.</p>{/if}

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
