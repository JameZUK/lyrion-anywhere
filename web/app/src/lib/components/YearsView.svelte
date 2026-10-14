<script lang="ts">
	// Years (also a section of Library): each opens its albums.
	import { untrack } from 'svelte';
	import { api, app } from '$lib/state/app.svelte';
	import { library } from '$lib/state/library.svelte';
	import * as cmd from '$lib/lms/commands';
	import { Pager, whenNear } from '$lib/lms/pager.svelte';
	import type { YearsResult } from '$lib/lms/types';
	import LibraryNote from './LibraryNote.svelte';

	/** Inside Library (a section of it): an h2 title, no page of its own. */
	let { embedded = false }: { embedded?: boolean } = $props();

	let sentinel = $state<HTMLDivElement>();

	/** Years as numbers; anything else LMS sends is left out. */
	function toYear(v: unknown): number | null {
		try {
			return cmd.year(v);
		} catch {
			return null;
		}
	}
	const query = (lib: string | null) => async (from: number) => {
		const r = await api.rpc<YearsResult>('', cmd.years(from, 500, lib));
		return { rows: (r.years_loop ?? []).map((y) => toYear(y?.year)).filter((y): y is number => y !== null), count: r.count };
	};
	const list = new Pager<number>(query(null), (y) => y);

	$effect(() => {
		const lib = library.id;
		void app.libraryVersion;
		untrack(() => void list.reset(query(lib)));
	});
	$effect(() => whenNear(sentinel, () => list.more()));
</script>

<div class="view" data-view="YearsView">
	{#if embedded}<h2 class="sr-only">Years</h2>{:else}<h1>Years</h1>{/if}
	<LibraryNote />
	{#if list.error}<p class="error">{list.error}</p>{/if}
	<ul class="years">
		{#each list.items as y (y)}
			<li><a href={cmd.yearLink(y) ?? '#/years'}>{y === 0 ? 'No year' : y}</a></li>
		{/each}
	</ul>
	{#if list.total === 0}<p class="muted empty">No years in the library.</p>{/if}
	{#if !list.done}<div bind:this={sentinel} aria-hidden="true"></div>{/if}
</div>

<style>
	.years {
		list-style: none;
		padding: 0;
		margin: 1rem 0 0;
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(5.5rem, 1fr));
		gap: 0.5rem;
		max-width: 900px;
	}
	a {
		display: block;
		text-align: center;
		padding: 0.6rem 0.4rem;
		border-radius: var(--radius-sm);
		background: var(--bg-raised);
		border: 1px solid var(--line);
		font-weight: 600;
		font-variant-numeric: tabular-nums;
	}
	a:hover {
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
	}
	.empty {
		padding: 1.5rem 0;
	}
</style>
