<script lang="ts">
	// Home: the user's shelves (docs/plans/wave1-design.md §7), in their own order, the same
	// on every device. Edit mode reorders (drag, or Move up/down), hides and adds shelves.
	import { me } from '$lib/state/me';
	import { api } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { textOf } from '$lib/lms/format';
	import type { HomeLayout, HomeShelf as Shelf, PinnedMenu, ShelfType, Station } from '$lib/api/me';
	import * as L from '$lib/home/layout';
	import HomeShelf from '$lib/components/HomeShelf.svelte';
	import Icon from '$lib/components/Icon.svelte';

	let layout = $state<HomeLayout | null>(null);
	let stations = $state<Station[]>([]);
	let menus = $state<PinnedMenu[]>([]);
	let error = $state('');
	let editing = $state(false);
	/** The layout as saved, to go back to if a save fails or editing is cancelled. */
	let saved: HomeLayout | null = null;
	let saving = $state(false);
	let message = $state('');
	let adding = $state<ShelfType | ''>('');
	let addId = $state<number | ''>('');
	let genres = $state<{ id: number; genre: string }[]>([]);
	let dragFrom = $state<number | null>(null);

	async function load() {
		error = '';
		try {
			const [l, s, m] = await Promise.all([me.home(), me.stations(), me.menus().catch(() => [] as PinnedMenu[])]);
			layout = l;
			saved = l;
			stations = s;
			menus = m;
		} catch (e) {
			error = (e as Error).message;
		}
	}
	void load();

	const label = (s: Shelf) =>
		s.type === 'station'
			? `Station: ${stations.find((x) => x.id === s.id)?.name ?? s.id}`
			: s.type === 'menu'
				? `Pinned: ${menus.find((x) => x.id === s.id)?.name ?? s.id}`
				: s.type === 'genre'
					? `Genre: ${genres.find((x) => x.id === s.id)?.genre ?? s.id}`
					: L.SHELF_LABELS[s.type];

	function change(next: HomeLayout) {
		layout = next;
		message = '';
	}

	function startEditing() {
		saved = layout;
		editing = true;
		message = '';
	}

	function cancel() {
		layout = saved;
		editing = false;
		message = '';
	}

	async function save() {
		if (!layout) return;
		const clean = L.clean(layout);
		const problem = L.layoutProblem(clean);
		if (problem) {
			message = `Can’t save this Home: ${problem}.`;
			return;
		}
		saving = true;
		try {
			layout = await me.saveHome(clean);
			saved = layout;
			editing = false;
			message = '';
		} catch (e) {
			layout = saved;
			editing = false;
			message = `Couldn’t save your Home: ${(e as Error).message}`;
		} finally {
			saving = false;
		}
	}

	async function reset() {
		if (!confirm('Put Home back to how it started?')) return;
		try {
			layout = await me.resetHome();
			saved = layout;
			editing = false;
		} catch (e) {
			message = (e as Error).message;
		}
	}

	async function pickType() {
		addId = '';
		if (adding === 'genre' && !genres.length) {
			try {
				const r = await api.rpc<{ genres_loop?: { id: number; genre?: string }[] }>('', cmd.genres(0, 300));
				genres = (r.genres_loop ?? []).map((g) => ({ id: g.id, genre: textOf(g.genre) }));
			} catch (e) {
				message = (e as Error).message;
			}
		}
	}

	function addShelf() {
		if (!layout || !adding) return;
		const withId = L.WITH_ID.includes(adding);
		if (withId && addId === '') {
			message = 'Choose which one to add.';
			return;
		}
		try {
			change(L.add(layout, withId ? { type: adding, id: Number(addId) } : { type: adding }));
			adding = '';
			addId = '';
		} catch (e) {
			message = (e as Error).message;
		}
	}

	// Pointer drag by the handle; Move up/down does the same for keyboards and screen readers.
	function dragStart(e: PointerEvent, i: number) {
		dragFrom = i;
		(e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
	}
	function dragMove(e: PointerEvent) {
		if (dragFrom === null || !layout) return;
		const el = document.elementFromPoint?.(e.clientX, e.clientY)?.closest('[data-shelf-index]') as HTMLElement | null;
		const to = el ? Number(el.dataset.shelfIndex) : NaN;
		if (Number.isInteger(to) && to !== dragFrom) {
			change(L.move(layout, dragFrom, to));
			dragFrom = to;
		}
	}
	function dragEnd() {
		dragFrom = null;
	}

	const choices = $derived(layout ? L.addable(layout) : []);
</script>

<section class="page home">
	<header>
		<h1>Home</h1>
		{#if layout && !editing}
			<button class="btn" onclick={startEditing}><Icon name="settings" size={18} />Edit Home</button>
		{/if}
	</header>
	{#if message}<p class="banner" role="status">{message}</p>{/if}
	{#if error}
		<p class="error">{error}</p>
		<button class="btn" onclick={load}>Try again</button>
	{:else if !layout}
		<p class="muted">Loading…</p>
	{:else if editing}
		<p class="muted">Drag a shelf by its handle, or use Move up and Move down. Hidden shelves stay here for later.</p>
		<ol class="edit" aria-label="Home shelves">
			{#each layout.shelves as s, i (L.shelfKey(s))}
				<li data-shelf-index={i} class:dragging={dragFrom === i} class:hidden={s.hidden}>
					<span
						class="handle"
						role="presentation"
						onpointerdown={(e) => dragStart(e, i)}
						onpointermove={dragMove}
						onpointerup={dragEnd}
						onpointercancel={dragEnd}><Icon name="grip" size={18} /></span
					>
					<span class="name">{label(s)}{s.hidden ? ' (hidden)' : ''}</span>
					<button class="icon-btn" aria-label={`Move ${label(s)} up`} disabled={i === 0} onclick={() => change(L.moveUp(layout!, i))}><Icon name="up" size={16} /></button>
					<button class="icon-btn" aria-label={`Move ${label(s)} down`} disabled={i === layout.shelves.length - 1} onclick={() => change(L.moveDown(layout!, i))}><Icon name="down" size={16} /></button>
					<button class="btn small" aria-label={`${s.hidden ? 'Show' : 'Hide'} ${label(s)}`} onclick={() => change(L.toggleHidden(layout!, i))}>{s.hidden ? 'Show' : 'Hide'}</button>
					<button class="icon-btn" aria-label={`Remove ${label(s)}`} onclick={() => change(L.remove(layout!, i))}><Icon name="trash" size={16} /></button>
				</li>
			{/each}
		</ol>
		<div class="add">
			<label class="field"
				><span>Add a shelf</span>
				<select bind:value={adding} onchange={pickType}>
					<option value="">Choose…</option>
					{#each choices as t (t)}<option value={t}>{L.SHELF_LABELS[t]}</option>{/each}
				</select>
			</label>
			{#if adding === 'station' || adding === 'menu' || adding === 'genre'}
				<label class="field"
					><span>Which</span>
					<select bind:value={addId}>
						<option value="">Choose…</option>
						{#if adding === 'station'}{#each stations as s (s.id)}<option value={s.id}>{s.name}</option>{/each}{/if}
						{#if adding === 'menu'}{#each menus as m (m.id)}<option value={m.id}>{m.name}</option>{/each}{/if}
						{#if adding === 'genre'}{#each genres as g (g.id)}<option value={g.id}>{g.genre}</option>{/each}{/if}
					</select>
				</label>
			{/if}
			<button class="btn" onclick={addShelf} disabled={!adding}><Icon name="plus" size={18} />Add</button>
		</div>
		<div class="actions">
			<button class="btn primary" onclick={save} disabled={saving}>Save</button>
			<button class="btn" onclick={cancel}>Cancel</button>
			<button class="btn" onclick={reset}>Reset to default</button>
		</div>
	{:else}
		{#each layout.shelves.filter((s) => !s.hidden) as s (L.shelfKey(s))}
			<HomeShelf shelf={s} {stations} {menus} onstationremoved={(id) => (stations = stations.filter((x) => x.id !== id))} />
		{:else}
			<p class="muted">Your Home is empty. Use Edit Home to add shelves.</p>
		{/each}
	{/if}
	<a class="browse" href="#/library"><Icon name="library" size={20} />Browse the library<span class="chev" aria-hidden="true">›</span></a>
</section>

<style>
	header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 0.5rem 1rem;
		flex-wrap: wrap;
		margin-bottom: 0.75rem;
	}
	h1 {
		margin: 0;
	}
	.banner {
		padding: 0.5rem 0.75rem;
		border-radius: var(--radius-sm);
		background: var(--accent-soft);
	}
	.edit {
		list-style: none;
		margin: 0.5rem 0 1rem;
		padding: 0;
		display: grid;
		gap: 0.4rem;
		max-width: 640px;
	}
	.edit li {
		display: flex;
		align-items: center;
		gap: 0.4rem;
		padding: 0.4rem 0.5rem;
		border-radius: var(--radius-sm);
		background: var(--bg-raised);
		border: 1px solid var(--line);
	}
	.edit li.dragging {
		border-color: var(--accent);
	}
	.edit li.hidden .name {
		color: var(--fg-muted);
	}
	.handle {
		cursor: grab;
		touch-action: none;
		padding: 0.3rem;
		color: var(--fg-muted);
	}
	.name {
		flex: 1;
		min-width: 0;
	}
	.small {
		padding: 0.3em 0.7em;
		font-size: 0.85rem;
	}
	.add,
	.actions {
		display: flex;
		flex-wrap: wrap;
		align-items: end;
		gap: 0.6rem;
		margin-bottom: 1rem;
	}
	.browse {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		max-width: 560px;
		margin-top: 1rem;
		padding: 0.8rem 0.9rem;
		border-radius: var(--radius);
		background: var(--bg-raised);
		border: 1px solid var(--line);
		font-weight: 600;
	}
	.chev {
		margin-left: auto;
		color: var(--fg-muted);
		font-size: 1.4rem;
	}
</style>
