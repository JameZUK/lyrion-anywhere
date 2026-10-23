<script lang="ts">
	// Genres (also a section of Library): find one, open it, or play it.
	import { untrack } from 'svelte';
	import { api, app } from '#lib/state/app.svelte';
	import { library } from '#lib/state/library.svelte';
	import * as cmd from '#lib/lms/commands';
	import { textOf } from '#lib/lms/format';
	import { Pager, whenNear } from '#lib/lms/pager.svelte';
	import type { Genre, GenresResult } from '#lib/lms/types';
	import Icon from './Icon.svelte';
	import LibraryNote from './LibraryNote.svelte';

	/** Inside Library (a section of it): an h2 title, no page of its own. */
	let { embedded = false }: { embedded?: boolean } = $props();

	const PAGE = 200;
	let filter = $state('');
	let sentinel = $state<HTMLDivElement>();

	const query = (lib: string | null, term: string) => async (from: number) => {
		const r = await api.rpc<GenresResult>('', cmd.genres(from, PAGE, lib, term));
		return { rows: r.genres_loop ?? [], count: r.count };
	};
	const list = new Pager<Genre>(query(null, ''), (g) => (typeof g.id === 'number' ? g.id : null));

	$effect(() => {
		const lib = library.id;
		const term = filter.trim();
		void app.libraryVersion;
		// Searching as you type: wait for a pause.
		const t = setTimeout(() => untrack(() => void list.reset(query(lib, term))), term ? 250 : 0);
		return () => clearTimeout(t);
	});
	$effect(() => whenNear(sentinel, () => list.more()));
</script>

<div class="view" data-view="GenresView">
	{#if embedded}<h2 class="sr-only">Genres</h2>{:else}<h1>Genres</h1>{/if}
	<LibraryNote />
	<label class="filter"><Icon name="search" size={18} /><span class="sr-only">Find a genre</span><input type="search" bind:value={filter} placeholder="Find a genre" maxlength="200" /></label>
	{#if list.total !== null}<p class="muted">{list.total.toLocaleString('en-GB')} {list.total === 1 ? 'genre' : 'genres'}</p>{/if}
	{#if list.error}<p class="error">{list.error}</p>{/if}
	<ul class="list">
		{#each list.items as g (g.id)}
			<li>
				<a class="main" href={cmd.link('genre', g.id) ?? '#/genres'}>
					<span class="glyph" aria-hidden="true"><Icon name="genre" size={20} /></span>
					<span class="name ellipsis">{textOf(g.genre)}</span>
				</a>
				<button class="icon-btn" aria-label={`Play ${textOf(g.genre)}`} onclick={() => app.queueUp({ genre: g.id }, 'play')} disabled={!app.canQueue}><Icon name="play" size={18} /></button>
			</li>
		{/each}
	</ul>
	{#if list.total === 0}<p class="muted empty">{filter.trim() ? 'No genres match.' : 'No genres in the library.'}</p>{/if}
	{#if !list.done}<div bind:this={sentinel} aria-hidden="true"></div>{/if}
</div>

<style>
	.filter {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		max-width: 420px;
		margin: 0.5rem 0 0.75rem;
		padding: 0.45rem 0.8rem;
		border: 1px solid var(--line);
		border-radius: 999px;
		background: var(--bg-raised);
		color: var(--fg-muted);
	}
	/* The input draws no ring of its own: the pill around it does (Wave 3.5 a11y). */
	.filter:focus-within {
		border-color: var(--accent);
		outline: 2px solid var(--accent);
		outline-offset: 2px;
	}
	.filter input {
		flex: 1;
		min-width: 0;
		border: 0;
		background: transparent;
		outline: none;
		color: var(--fg);
	}
	.list {
		list-style: none;
		margin: 1rem 0 0;
		padding: 0;
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(min(100%, 280px), 1fr));
		gap: 0.2rem 1rem;
	}
	li {
		display: flex;
		align-items: center;
		border-radius: var(--radius-sm);
	}
	li:hover {
		background: var(--bg-sunken);
	}
	.main {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 0.75rem;
		padding: 0.5rem 0.6rem;
	}
	.glyph {
		display: grid;
		place-items: center;
		width: 36px;
		height: 36px;
		border-radius: var(--radius-sm);
		background: var(--accent-soft);
		color: var(--accent);
		flex: none;
	}
	.name {
		font-weight: 550;
	}
	.empty {
		padding: 1.5rem 0;
	}
</style>
