<script lang="ts" module>
	import type { Role } from '#lib/lms/commands';
	export type ArtistView = Role | 'all';
	/** Kept while the app is open, so coming back from an artist shows the same list. */
	let lastView: ArtistView = 'ALBUMARTIST';
</script>

<script lang="ts">
	// Artists by role (album artists first, as in LMS; other roles only where the library has
	// them), paged as it scrolls, with the A–Z fast scroller from LMS's index.
	import { tick, untrack } from 'svelte';
	import { api, app } from '#lib/state/app.svelte';
	import { library } from '#lib/state/library.svelte';
	import * as cmd from '#lib/lms/commands';
	import type { IndexEntry } from '#lib/lms/azindex';
	import { WindowPager, whenNear } from '#lib/lms/pager.svelte';
	import type { Artist, ArtistsResult } from '#lib/lms/types';
	import ArtistList from './ArtistList.svelte';
	import AzScroller from './AzScroller.svelte';
	import Chips from './Chips.svelte';
	import Icon from './Icon.svelte';

	const PAGE = 200;
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
	let root = $state<HTMLDivElement>();
	const panel = `artists-${Math.random().toString(36).slice(2, 9)}`;

	const query = (v: ArtistView, lib: string | null) => async (from: number, n: number, first: boolean) => {
		const r = await api.rpc<ArtistsResult & { indexList?: unknown }>('', cmd.artists(from, n, undefined, { role: v === 'all' ? undefined : v, library: lib }, first));
		return { rows: r?.artists_loop ?? [], count: r?.count, ...(first ? { indexList: r?.indexList } : {}) };
	};
	const list = new WindowPager<Artist>(query(lastView, null), (a) => (typeof a?.id === 'number' ? a.id : null), PAGE);

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
					if (!live || !(Number(r?.count) > 0)) return;
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

	async function jump(e: IndexEntry) {
		await list.jumpTo(e.offset);
		await tick();
		const target = root?.querySelector<HTMLElement>(`[data-pos="${e.offset}"] a`);
		target?.scrollIntoView?.({ block: 'start' });
		target?.focus({ preventScroll: true });
	}
</script>

<div bind:this={root}>
	<Chips items={shown.map((v) => ({ value: v.view, label: v.label }))} value={view} onchange={(v) => (view = v)} label="Show" {panel} />
	<div id={panel} role="tabpanel" aria-label={shown.find((v) => v.view === view)?.label ?? 'Artists'}>
		{#if list.total !== null}<p class="muted">{list.total.toLocaleString('en-GB')} {list.total === 1 ? 'artist' : 'artists'}</p>{/if}
		{#if list.error}<p class="error">{list.error}</p>{/if}
		<AzScroller index={list.index} onjump={jump} noun="artists" />
		{#if list.base > 0}
			<button class="btn earlier" onclick={() => list.earlier()} data-earlier><Icon name="up" size={18} />Show earlier artists</button>
		{/if}
		<ArtistList artists={list.items} offset={list.base} />
		{#if list.total === 0}<p class="muted empty">No artists here.</p>{/if}
		{#if !list.done}<div bind:this={sentinel} aria-hidden="true"></div>{/if}
	</div>
</div>

<style>
	.earlier {
		margin: 0.5rem 0;
	}
	.empty {
		padding: 1.5rem 0;
	}
</style>
