<script lang="ts" module>
	import type { Role } from '$lib/lms/commands';
	export type ArtistView = Role | 'all';
	/** Kept while the app is open, so coming back from an artist shows the same list. */
	let lastView: ArtistView = 'ALBUMARTIST';
</script>

<script lang="ts">
	import { untrack } from 'svelte';
	import { api, app } from '$lib/state/app.svelte';
	import { library } from '$lib/state/library.svelte';
	import * as cmd from '$lib/lms/commands';
	import { Pager, whenNear } from '$lib/lms/pager.svelte';
	import type { Artist, ArtistsResult } from '$lib/lms/types';
	import ArtistList from '$lib/components/ArtistList.svelte';
	import LibraryNote from '$lib/components/LibraryNote.svelte';

	const PAGE = 200;
	// Album artists first, as in LMS; the other roles only where the library has them.
	const VIEWS: { view: ArtistView; label: string; always: boolean }[] = [
		{ view: 'ALBUMARTIST', label: 'Album artists', always: true },
		{ view: 'all', label: 'All artists', always: true },
		{ view: 'COMPOSER', label: 'Composers', always: false },
		{ view: 'CONDUCTOR', label: 'Conductors', always: false },
		{ view: 'BAND', label: 'Bands', always: false },
	];
	let view = $state<ArtistView>(lastView);
	let present = $state<Set<ArtistView>>(new Set());
	let sentinel = $state<HTMLDivElement>();

	const query = (v: ArtistView, lib: string | null) => async (from: number) => {
		const r = await api.rpc<ArtistsResult>('', cmd.artists(from, PAGE, undefined, { role: v === 'all' ? undefined : v, library: lib }));
		return { rows: r.artists_loop ?? [], count: r.count };
	};
	const list = new Pager<Artist>(query(lastView, null),(a) => (typeof a.id === 'number' ? a.id : null));

	$effect(() => {
		const v = view;
		const lib = library.id;
		void app.libraryVersion;
		lastView = v;
		untrack(() => void list.reset(query(v, lib)));
	});

	// Which optional roles the library has (one row each is enough to tell).
	$effect(() => {
		const lib = library.id;
		void app.libraryVersion;
		let live = true;
		for (const { view: v, always } of VIEWS) {
			if (always || v === 'all') continue;
			api.rpc<ArtistsResult>('', cmd.hasRole(v, lib)).then(
				(r) => {
					if (!live || !(Number(r.count) > 0)) return;
					present = new Set([...present, v]);
				},
				() => {},
			);
		}
		return () => {
			live = false;
			present = new Set();
		};
	});

	const shown = $derived(VIEWS.filter((v) => v.always || present.has(v.view) || v.view === view));
	$effect(() => whenNear(sentinel, () => list.more()));
</script>

<section class="page">
	<h1>Artists</h1>
	<LibraryNote />
	<div class="chips" role="tablist" aria-label="Show">
		{#each shown as v (v.view)}
			<button role="tab" aria-selected={view === v.view} class:on={view === v.view} onclick={() => (view = v.view)}>{v.label}</button>
		{/each}
	</div>
	{#if list.total !== null}<p class="muted">{list.total.toLocaleString('en-GB')} {list.total === 1 ? 'artist' : 'artists'}</p>{/if}
	{#if list.error}<p class="error">{list.error}</p>{/if}
	<ArtistList artists={list.items} />
	{#if list.total === 0}<p class="muted empty">No artists here.</p>{/if}
	{#if !list.done}<div bind:this={sentinel} aria-hidden="true"></div>{/if}
</section>

<style>
	.chips {
		display: flex;
		flex-wrap: wrap;
		gap: 0.4rem;
		margin: 0.5rem 0 0.5rem;
	}
	.chips button {
		border: 1px solid var(--line);
		background: var(--bg-raised);
		color: var(--fg-muted);
		font: inherit;
		font-weight: 600;
		font-size: 0.9rem;
		padding: 0.35rem 0.9rem;
		border-radius: 999px;
		cursor: pointer;
	}
	.chips button.on {
		background: var(--accent-soft);
		border-color: transparent;
		color: var(--accent-soft-fg);
	}
	.empty {
		padding: 1.5rem 0;
	}
</style>
