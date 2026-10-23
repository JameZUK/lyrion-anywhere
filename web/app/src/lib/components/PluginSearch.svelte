<script lang="ts">
	// "Search on …" under the library results: one link per app whose top menu has a search
	// box, opening that app's own results for the same text (in the plugin menu browser, where
	// play and add work as for any app item). Which apps can be searched is found once per
	// session (see `pluginSearch`); nothing is hard-coded per app.
	import { untrack } from 'svelte';
	import { app } from '#lib/state/app.svelte';
	import { pluginSearch } from '#lib/state/plugin-search';
	import { searchOnHref, type SearchablePlugin } from '#lib/search/plugins';
	import * as cmd from '#lib/lms/commands';
	import Art from './Art.svelte';
	import Icon from './Icon.svelte';

	let { query, onpick }: { query: string; /** A row was chosen (for recent searches). */ onpick?: () => void } = $props();

	let found = $state<SearchablePlugin[]>([]);
	/** The player the list was asked for (asked once per page; the directory caches it). */
	let askedFor = '';

	$effect(() => {
		const p = app.playerId ?? '';
		const wanted = !!query.trim();
		untrack(() => {
			if (!wanted || !p) return;
			if (p === askedFor) return;
			askedFor = p;
			pluginSearch
				.get(p)
				.then((list) => {
					if (app.playerId === p) found = list;
				})
				.catch(() => {
					found = []; // not essential: the library results still show
				});
		});
	});
	const q = $derived(query.trim());
</script>

{#if q && found.length}
	<section class="on" aria-labelledby="search-on-h" data-search-on>
		<h2 id="search-on-h">Search on…</h2>
		<ul>
			{#each found as p (p.cmd)}
				<li>
					<a href={searchOnHref(p.cmd, q)} onclick={() => onpick?.()} data-search-on-app={p.cmd}>
						<span class="pic">{#if p.icon}<Art path={cmd.imagePath(p.icon, 96)} alt="" label={p.name} />{:else}<Icon name="search" size={20} />{/if}</span>
						<span class="ellipsis">Search {p.name} for “{q}”</span>
						<span class="chev" aria-hidden="true">›</span>
					</a>
				</li>
			{/each}
		</ul>
	</section>
{/if}

<style>
	h2 {
		margin: 1.25rem 0 0.4rem;
		font-size: 1.05rem;
	}
	ul {
		list-style: none;
		padding: 0;
		margin: 0;
		display: grid;
		gap: 0.1rem;
		max-width: 760px;
	}
	a {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		min-height: 48px;
		padding: 0.3rem 0.6rem;
		border-radius: var(--radius-sm);
		color: var(--fg);
	}
	a:hover,
	a:focus-visible {
		background: var(--bg-sunken);
	}
	.pic {
		width: 40px;
		flex: none;
		display: grid;
		place-items: center;
		color: var(--fg-muted);
	}
	.ellipsis {
		flex: 1;
		min-width: 0;
	}
	.chev {
		color: var(--fg-muted);
		font-size: 1.3rem;
	}
</style>
