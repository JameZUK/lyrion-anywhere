<script lang="ts">
	// One smart filter (wave2-design.md §5.6): its albums, a page at a time, run on the gateway;
	// play or shuffle its first albums, edit or delete it.
	import BackButton from '#lib/components/BackButton.svelte';
	import AlbumGrid from '#lib/components/AlbumGrid.svelte';
	import FilterEditor from '#lib/components/FilterEditor.svelte';
	import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
	import Icon from '#lib/components/Icon.svelte';
	import RowMenu from '#lib/components/RowMenu.svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { untrack } from 'svelte';
	import { useDetailTitle } from '#lib/title.svelte';
	import { me } from '#lib/state/me';
	import { app } from '#lib/state/app.svelte';
	import { ruleSummary, validId, type FilterSpec, type SmartFilter } from '#lib/api/filters';
	import { albumsFor, startFilter } from '#lib/filters/run';
	import type { Album } from '#lib/lms/types';

	const PAGE = 50;
	const filterId = $derived.by(() => {
		const n = Number(page.params.id);
		return /^\d{1,12}$/.test(page.params.id ?? '') && validId(n) ? n : null;
	});

	let filter = $state<SmartFilter | null>(null);
	let albums = $state<Album[]>([]);
	let total = $state<number | null>(null);
	let next = $state(0);
	let busy = false;
	let error = $state('');
	let message = $state('');
	let editing = $state(false);
	let deleting = $state(false);

	async function loadFilter(id: number) {
		try {
			const f = (await me.filters()).find((x) => x.id === id) ?? null;
			if (filterId !== id) return;
			filter = f;
			if (!f) error = 'This smart playlist has been deleted, or isn’t yours.';
		} catch (e) {
			error = (e as Error).message;
		}
	}

	async function more() {
		const id = filterId;
		if (id === null || busy || (total !== null && next >= total)) return;
		busy = true;
		try {
			const r = await me.filterAlbums(id, next, PAGE);
			const rows = await albumsFor(r.albums);
			if (filterId !== id) return;
			albums = [...albums, ...rows.filter((a) => !albums.some((b) => b.id === a.id))];
			total = r.count;
			next += r.albums.length;
			if (!r.albums.length) total = next;
		} catch (e) {
			error = (e as Error).message;
		} finally {
			busy = false;
		}
	}

	function reset() {
		albums = [];
		total = null;
		next = 0;
		error = '';
	}

	$effect(() => {
		const id = filterId;
		void app.libraryVersion;
		untrack(() => {
			reset();
			filter = null;
			if (id === null) {
				error = 'Not a smart playlist.';
				return;
			}
			void loadFilter(id);
			void more();
		});
	});

	useDetailTitle(() => filter?.name ?? '');

	async function play(shuffle: boolean) {
		if (!filter) return;
		message = '';
		const r = await startFilter(filter.id, shuffle);
		message = r || `${shuffle ? 'Shuffling' : 'Playing'} “${filter.name}”.`;
	}

	async function save(spec: FilterSpec): Promise<string> {
		if (!filter) return '';
		try {
			await me.updateFilter(filter.id, spec);
		} catch (e) {
			return `Couldn’t save the smart playlist: ${(e as Error).message}`;
		}
		editing = false;
		filter = { ...spec, id: filter.id };
		message = `Saved “${spec.name}”.`;
		reset();
		void more();
		return '';
	}

	async function remove() {
		deleting = false;
		if (!filter) return;
		try {
			await me.deleteFilter(filter.id);
			await goto('#/library');
		} catch (e) {
			message = `Couldn’t delete “${filter.name}”: ${(e as Error).message}`;
		}
	}
</script>

<section class="page">
	<BackButton parent="/library" />
	<p class="kind muted">Smart playlist</p>
	<h1>{filter?.name ?? (error ? 'Smart playlist' : '…')}</h1>
	{#if filter}
		<p class="muted rules">{filter.rules.map((r) => ruleSummary(r)).join(' · ')}</p>
		<div class="actions">
			<button class="btn primary" onclick={() => play(false)} disabled={total === 0} data-filter-play><Icon name="play" size={18} />Play</button>
			<button class="btn" onclick={() => play(true)} disabled={total === 0} data-filter-shuffle><Icon name="shuffle" size={18} />Shuffle</button>
			<RowMenu
				label={`More options for ${filter.name}`}
				actions={[
					{ label: 'Edit', icon: 'pencil', run: () => (editing = true), id: 'edit' },
					{ label: 'Delete', icon: 'trash', run: () => (deleting = true), danger: true, id: 'delete' },
				]}
				trigger={{ 'data-page-more': true }}
			/>
		</div>
		<p class="muted small">Play and Shuffle use the first {Math.min(50, total ?? 50)} albums.</p>
	{/if}
	<p class="status" role="status">{message}</p>
	{#if error}<p class="error">{error}</p>{/if}
	{#if total !== null}<p class="muted">{total.toLocaleString('en-GB')} {total === 1 ? 'album' : 'albums'}</p>{/if}
	{#if total === 0}
		<p class="muted">No albums match this smart playlist.</p>
	{:else}
		<AlbumGrid {albums} more={total !== null && next < total ? more : null} />
	{/if}
</section>

{#if editing && filter}
	<FilterEditor initial={filter} onsave={save} oncancel={() => (editing = false)} />
{/if}
{#if deleting && filter}
	<ConfirmDialog title={`Delete “${filter.name}”?`} message="The smart playlist is removed from every device. Your music isn’t touched." confirmLabel="Delete" danger onconfirm={remove} oncancel={() => (deleting = false)} />
{/if}

<style>
	.kind {
		margin: 0;
		font-size: 0.85rem;
		text-transform: uppercase;
		letter-spacing: 0.06em;
	}
	h1 {
		margin: 0.1rem 0 0.25rem;
	}
	.rules {
		margin: 0 0 0.75rem;
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.5rem;
	}
	.actions .btn {
		min-height: 44px;
	}
	.small {
		font-size: 0.85rem;
	}
	.status:empty {
		display: none;
	}
</style>
