<script lang="ts">
	// Library search (Wave 2 §3.3). Recent searches (on this device only) while the box is
	// empty; then one search grouped into a Top result, artists, albums, tracks and playlists,
	// with type chips that page through one type alone. The term, results and chip live in
	// `search` (not the page), so going to a result and back finds them as they were.
	import { untrack } from 'svelte';
	import { api, session } from '$lib/state/app.svelte';
	import { search } from '$lib/state/search';
	import * as cmd from '$lib/lms/commands';
	import { Pager, whenNear } from '$lib/lms/pager.svelte';
	import { MIN_TERM } from '$lib/search/store.svelte';
	import { addRecent, loadRecents, removeRecent, saveRecents } from '$lib/search/recents';
	import { EMPTY_GROUPS, SEARCH_TYPES, TYPE_LABELS, hits, topResult, totalHits, type Hit, type SearchType } from '$lib/search/results';
	import Chips from '$lib/components/Chips.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import SearchHit from '$lib/components/SearchHit.svelte';
	import PluginSearch from '$lib/components/PluginSearch.svelte';

	/** Rows of each type under All before "See all". */
	const PREVIEW = 5;
	const PAGE = 40;
	const panel = 'search-results';

	const user = session.current?.user.username ?? '';
	let recents = $state<string[]>(loadRecents(user));
	let box = $state<HTMLInputElement>();
	let sentinel = $state<HTMLDivElement>();

	const groups = $derived(search.result ?? EMPTY_GROUPS);
	const found = $derived(search.result ? totalHits(search.result) : 0);
	const empty = $derived(!!search.result && found === 0);
	const top = $derived(search.result ? topResult(search.result, search.shownFor) : null);
	const showRecents = $derived(search.term.trim().length < MIN_TERM && recents.length > 0);
	/** What a screen reader hears once a search is answered. */
	const said = $derived(search.busy || !search.result ? '' : empty ? `Nothing found for “${search.shownFor}”.` : `${found} ${found === 1 ? 'result' : 'results'} for “${search.shownFor}”.`);
	// Back from a result: don't pop the keyboard up over the results.
	const focusBox = !search.term;

	function remember(term = search.shownFor || search.term) {
		recents = addRecent(recents, term);
		saveRecents(user, recents);
	}
	function forget(term: string) {
		recents = removeRecent(recents, term);
		saveRecents(user, recents);
		box?.focus();
	}
	function clearRecents() {
		recents = [];
		saveRecents(user, recents);
		box?.focus();
	}
	function useRecent(term: string) {
		search.set(term);
		remember(term);
		box?.focus();
	}
	function onsubmit(e: SubmitEvent) {
		e.preventDefault();
		if (search.term.trim().length >= MIN_TERM) remember(search.term);
	}

	// One type alone: paged on its own query.
	type Fetch = (from: number) => Promise<{ rows: Hit[]; count?: number }>;
	function typeQuery(t: SearchType, q: string): Fetch {
		return async (from) => {
			if (t === 'artists') {
				const r = await api.rpc<{ count?: number; artists_loop?: unknown }>('', cmd.artists(from, PAGE, q));
				return { rows: hits(r?.artists_loop, 'artist', 'id', 'artist'), count: r?.count };
			}
			if (t === 'albums') {
				const r = await api.rpc<{ count?: number; albums_loop?: unknown }>('', cmd.albums(from, PAGE, 'album', q));
				return { rows: hits(r?.albums_loop, 'album', 'id', 'album'), count: r?.count };
			}
			if (t === 'tracks') {
				const r = await api.rpc<{ count?: number; titles_loop?: unknown }>('', cmd.searchTracks(q, from, PAGE));
				return { rows: hits(r?.titles_loop, 'track', 'id', 'title'), count: r?.count };
			}
			const r = await api.rpc<{ count?: number; playlists_loop?: unknown }>('', cmd.searchPlaylists(q, from, PAGE));
			return { rows: hits(r?.playlists_loop, 'playlist', 'id', 'playlist'), count: r?.count };
		};
	}
	const paged = new Pager<Hit>(async () => ({ rows: [] }), (h) => h.id);
	$effect(() => {
		const t = search.type;
		const q = search.shownFor;
		if (t === 'all' || !q) return;
		untrack(() => void paged.reset(typeQuery(t, q)));
	});
	$effect(() => whenNear(sentinel, () => paged.more()));

	const sections = $derived([
		{ type: 'artists' as const, rows: groups.artists, count: groups.counts.artists },
		{ type: 'albums' as const, rows: groups.albums, count: groups.counts.albums },
		{ type: 'tracks' as const, rows: groups.tracks, count: groups.counts.tracks },
		{ type: 'playlists' as const, rows: groups.playlists, count: groups.counts.playlists },
	]);
</script>

<section class="page">
	<h1>Search</h1>
	<form role="search" {onsubmit}>
		<label class="box">
			<Icon name="search" />
			<span class="sr-only">Search the library</span>
			<!-- svelte-ignore a11y_autofocus -->
			<input
				bind:this={box}
				type="search"
				value={search.term}
				oninput={(e) => search.set(e.currentTarget.value)}
				placeholder="Artists, albums, tracks and playlists"
				autocomplete="off"
				spellcheck="false"
				enterkeyhint="search"
				autofocus={focusBox}
				data-search-input
			/>
			{#if search.busy}<span class="spin" role="img" aria-label="Searching"></span>{/if}
		</label>
	</form>
	{#if search.error}<p class="error" role="alert">{search.error}</p>{/if}
	<p class="sr-only" role="status">{said}</p>

	{#if showRecents}
		<section class="recents" aria-labelledby="recent-h" data-recents>
			<div class="head">
				<h2 id="recent-h">Recent searches</h2>
				<button class="btn" onclick={clearRecents} data-recents-clear>Clear all</button>
			</div>
			<p class="muted note">Kept on this device only.</p>
			<ul>
				{#each recents as r (r)}
					<li>
						<button class="term" onclick={() => useRecent(r)}><Icon name="search" size={16} /><span class="ellipsis">{r}</span></button>
						<button class="icon-btn" aria-label={`Remove “${r}” from recent searches`} onclick={() => forget(r)}><Icon name="close" size={16} /></button>
					</li>
				{/each}
			</ul>
		</section>
	{/if}

	{#if search.result && !empty}
		<Chips items={SEARCH_TYPES.map((t) => ({ value: t, label: TYPE_LABELS[t] }))} value={search.type} onchange={(t) => (search.type = t)} label="Show results" idPrefix="search-type" {panel} />
	{/if}
	{#if empty}<p class="muted" aria-hidden="true">Nothing found for “{search.shownFor}”.</p>{/if}

	<div id={panel} role={search.result && !empty ? 'tabpanel' : undefined} aria-labelledby={search.result && !empty ? `search-type-${search.type}` : undefined}>
		{#if search.result && !empty && search.type === 'all'}
			{#if top}
				<h2>Top result</h2>
				<div class="top"><SearchHit hit={top} top onpick={() => remember()} /></div>
			{/if}
			{#each sections as s (s.type)}
				{#if s.rows.length}
					<div class="group" data-group={s.type}>
						<h2>{TYPE_LABELS[s.type]}</h2>
						<ul class="list">
							{#each s.rows.slice(0, PREVIEW) as h (`${h.kind}${h.id}`)}
								<li><SearchHit hit={h} onpick={() => remember()} /></li>
							{/each}
						</ul>
						{#if s.count > PREVIEW}
							<button class="btn more" onclick={() => (search.type = s.type)} data-see-all={s.type}>See all {s.count.toLocaleString('en-GB')} {TYPE_LABELS[s.type].toLowerCase()}</button>
						{/if}
					</div>
				{/if}
			{/each}
		{:else if search.result && !empty}
			{#if paged.error}<p class="error">{paged.error}</p>{/if}
			{#if paged.total !== null}<p class="muted">{paged.total.toLocaleString('en-GB')} {TYPE_LABELS[search.type].toLowerCase()}</p>{/if}
			<ul class="list" data-type-list={search.type}>
				{#each paged.items as h (`${h.kind}${h.id}`)}
					<li><SearchHit hit={h} onpick={() => remember()} /></li>
				{/each}
			</ul>
			{#if paged.total === 0}<p class="muted">No {TYPE_LABELS[search.type].toLowerCase()} found.</p>{/if}
			{#if !paged.done}<div bind:this={sentinel} aria-hidden="true"></div>{/if}
		{/if}
	</div>
	<!-- The same text in each app that offers a search (below the library's own results). -->
	<PluginSearch query={search.type === 'all' && search.term.trim().length >= MIN_TERM ? search.term.trim() : ''} onpick={() => remember(search.term)} />
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
		margin-bottom: 1rem;
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
		margin: 1.25rem 0 0.4rem;
		font-size: 1.05rem;
	}
	.list,
	.recents ul {
		list-style: none;
		padding: 0;
		margin: 0;
		display: grid;
		gap: 0.1rem;
		max-width: 760px;
	}
	.top {
		max-width: 520px;
	}
	.more {
		margin-top: 0.4rem;
	}
	.recents .head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		max-width: 620px;
	}
	.recents .head h2 {
		margin: 0;
	}
	.note {
		font-size: 0.8rem;
		margin: 0.2rem 0 0.4rem;
	}
	.recents li {
		display: flex;
		align-items: center;
		max-width: 620px;
	}
	.term {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 0.7rem;
		min-height: 44px;
		padding: 0 0.6rem;
		border: 0;
		background: transparent;
		text-align: left;
		color: var(--fg);
		font: inherit;
		border-radius: var(--radius-sm);
	}
	.term:hover {
		background: var(--bg-sunken);
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
