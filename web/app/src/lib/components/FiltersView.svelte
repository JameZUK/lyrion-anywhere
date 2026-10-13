<script lang="ts">
	// Smart filters (wave2-design.md §5.6) in Library: the user's filters, each opened, played,
	// shuffled, edited or deleted; and a new one from the rule builder.
	import { me } from '$lib/state/me';
	import { FILTER_LIMITS, ruleSummary, type FilterSpec, type SmartFilter } from '$lib/api/filters';
	import { startFilter } from '$lib/filters/run';
	import FilterEditor from './FilterEditor.svelte';
	import ConfirmDialog from './ConfirmDialog.svelte';
	import Icon from './Icon.svelte';

	let { level = 2 }: { level?: 2 | 3 } = $props();

	let filters = $state<SmartFilter[] | null>(null);
	let error = $state('');
	let message = $state('');
	/** The filter being edited: null for a new one, undefined when the editor is closed. */
	let editing = $state<SmartFilter | null | undefined>(undefined);
	let deleting = $state<SmartFilter | null>(null);

	async function load() {
		error = '';
		try {
			filters = await me.filters();
		} catch (e) {
			error = (e as Error).message;
		}
	}
	void load();

	const summary = (f: SmartFilter) => f.rules.map((r) => ruleSummary(r)).join(' · ');

	async function save(spec: FilterSpec): Promise<string> {
		const was = editing;
		try {
			if (was) await me.updateFilter(was.id, spec);
			else await me.createFilter(spec);
		} catch (e) {
			return `Couldn’t save the filter: ${(e as Error).message}`;
		}
		editing = undefined;
		message = was ? `Saved “${spec.name}”.` : `Added “${spec.name}”.`;
		await load();
		return '';
	}

	async function remove() {
		const f = deleting;
		deleting = null;
		if (!f) return;
		try {
			await me.deleteFilter(f.id);
			message = `Deleted “${f.name}”.`;
			filters = (filters ?? []).filter((x) => x.id !== f.id);
		} catch (e) {
			message = `Couldn’t delete “${f.name}”: ${(e as Error).message}`;
		}
	}

	async function play(f: SmartFilter, shuffle: boolean) {
		message = '';
		const r = await startFilter(f.id, shuffle);
		message = r || `${shuffle ? 'Shuffling' : 'Playing'} “${f.name}”.`;
	}

	const full = $derived((filters?.length ?? 0) >= FILTER_LIMITS.maxFilters);
</script>

<div class="filters" data-filters-view>
	<div class="head">
		<svelte:element this={`h${level}`} class="title">Smart filters</svelte:element>
		<button class="btn primary" onclick={() => (editing = null)} disabled={full || filters === null} data-filter-new><Icon name="plus" size={18} />New filter</button>
	</div>
	<p class="muted small">Albums chosen by rules: genres, a decade, an artist, compilations. Kept for your account on every device{full ? `. You have the most allowed (${FILTER_LIMITS.maxFilters}).` : '.'}</p>
	<p class="status" role="status" data-filter-status>{message}</p>
	{#if error}
		<p class="error">{error}</p>
		<button class="btn" onclick={load}>Try again</button>
	{:else if filters === null}
		<p class="muted">Loading…</p>
	{:else if !filters.length}
		<p class="muted">No smart filters yet. Use New filter to make one.</p>
	{:else}
		<ul class="list">
			{#each filters as f (f.id)}
				<li data-filter={f.id}>
					<a class="main" href={`#/filter/${f.id}`}>
						<span class="glyph"><Icon name="filter" size={20} /></span>
						<span class="text"><span class="name ellipsis">{f.name}</span><span class="sub ellipsis muted">{summary(f)}</span></span>
					</a>
					<button class="icon-btn" aria-label={`Play ${f.name}`} onclick={() => play(f, false)} data-filter-play><Icon name="play" size={18} /></button>
					<button class="icon-btn" aria-label={`Shuffle ${f.name}`} onclick={() => play(f, true)} data-filter-shuffle><Icon name="shuffle" size={18} /></button>
					<button class="icon-btn" aria-label={`Edit ${f.name}`} onclick={() => (editing = f)} data-filter-edit><Icon name="settings" size={18} /></button>
					<button class="icon-btn" aria-label={`Delete ${f.name}`} onclick={() => (deleting = f)} data-filter-delete><Icon name="trash" size={18} /></button>
				</li>
			{/each}
		</ul>
	{/if}
</div>

{#if editing !== undefined}
	<FilterEditor initial={editing} onsave={save} oncancel={() => (editing = undefined)} />
{/if}
{#if deleting}
	<ConfirmDialog title={`Delete “${deleting.name}”?`} message="The filter is removed from every device. Your music isn’t touched." confirmLabel="Delete" danger onconfirm={remove} oncancel={() => (deleting = null)} />
{/if}

<style>
	.head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		flex-wrap: wrap;
		gap: 0.5rem;
	}
	.title {
		margin: 0;
		font-size: 1.15rem;
	}
	.small {
		font-size: 0.85rem;
	}
	.status:empty {
		display: none;
	}
	.list {
		list-style: none;
		margin: 0.5rem 0 0;
		padding: 0;
		display: grid;
		gap: 0.25rem;
		max-width: 760px;
	}
	li {
		display: flex;
		align-items: center;
		gap: 0.15rem;
		border-bottom: 1px solid var(--line);
	}
	.main {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 0.75rem;
		min-height: 56px;
		padding: 0.25rem 0.25rem;
		color: var(--fg);
	}
	.glyph {
		display: grid;
		place-items: center;
		width: 40px;
		height: 40px;
		flex: none;
		border-radius: var(--radius-sm);
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
	}
	.text {
		display: grid;
		min-width: 0;
	}
	.name {
		font-weight: 600;
	}
	.sub {
		font-size: 0.85rem;
	}
	li .icon-btn {
		width: 44px;
		height: 44px;
		flex: none;
	}
</style>
