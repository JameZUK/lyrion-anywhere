<script lang="ts">
	// Stations as cards: tap to start one on the selected player. Deleting one is in its ⋯
	// (Wave 3.5: destructive actions live only inside a menu), confirmed in a dialog.
	import type { Station, StationKind } from '#lib/api/me';
	import { activeStation } from '#lib/stations/active.svelte';
	import { me } from '#lib/state/me';
	import { app } from '#lib/state/app.svelte';
	import ConfirmDialog from './ConfirmDialog.svelte';
	import Icon from './Icon.svelte';
	import RowMenu from './RowMenu.svelte';

	let {
		stations,
		removable = false,
		onremoved,
	}: { stations: Station[]; removable?: boolean; onremoved?: (id: number) => void } = $props();

	const KIND: Record<StationKind, string> = {
		forgotten: 'Often played, not heard for a while',
		fresh: 'Recently added, barely played',
		deep_cuts: 'Rarely played, by artists you love',
		genre: 'Genre mix',
		decade: 'Decade mix',
		menu: 'From a menu',
	};
	let busy = $state<number | null>(null);
	let note = $state('');
	let deleting = $state<Station | null>(null);

	async function play(s: Station) {
		busy = s.id;
		note = '';
		const ok = await activeStation.start(s);
		busy = null;
		note = ok ? `Playing ${s.name}` : activeStation.error;
	}

	async function remove(s: Station) {
		deleting = null;
		try {
			await me.deleteStation(s.id);
			onremoved?.(s.id);
		} catch (e) {
			note = (e as Error).message;
		}
	}
</script>

{#if note}<p class="note" role="status">{note}</p>{/if}
{#if deleting}
	{@const s = deleting}
	<ConfirmDialog title={`Delete the station “${s.name}”?`} confirmLabel="Delete" danger onconfirm={() => remove(s)} oncancel={() => (deleting = null)} />
{/if}
<ul class="cards">
	{#each stations as s (s.id)}
		<li class:broken={!!s.broken} class:on={activeStation.playing && activeStation.station?.id === s.id}>
			<button class="play" onclick={() => play(s)} disabled={busy !== null || !app.canQueue} aria-label={`Play the station ${s.name}`}>
				<span class="glyph"><Icon name={s.kind === 'menu' ? 'radio' : 'shuffle'} size={22} /></span>
				<span class="words">
					<strong class="ellipsis">{s.name}</strong>
					<span class="muted small">{s.broken ? `Can't play: ${s.broken}` : KIND[s.kind] ?? ''}</span>
				</span>
				{#if busy === s.id}<span class="muted small">Starting…</span>{/if}
			</button>
			{#if removable}
				<RowMenu label={`More options for ${s.name}`} actions={[{ label: 'Delete station', icon: 'trash', run: () => (deleting = s), danger: true, id: 'delete-station' }]} />
			{/if}
		</li>
	{:else}
		<li class="empty muted">No stations yet.</li>
	{/each}
</ul>

<style>
	.cards {
		list-style: none;
		margin: 0;
		padding: 0.25rem 0 0.5rem;
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(min(100%, 240px), 1fr));
		gap: 0.6rem;
	}
	li {
		display: flex;
		align-items: center;
		border-radius: var(--radius);
		background: var(--bg-raised);
		border: 1px solid var(--line);
	}
	li.on {
		border-color: var(--accent);
	}
	li.broken .glyph {
		opacity: 0.5;
	}
	.play {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 0.75rem;
		padding: 0.7rem 0.8rem;
		border: 0;
		background: transparent;
		text-align: left;
		color: inherit;
	}
	.glyph {
		display: grid;
		place-items: center;
		width: 42px;
		height: 42px;
		flex: none;
		border-radius: var(--radius-sm);
		background: var(--accent-soft);
		color: var(--accent);
	}
	.words {
		display: grid;
		min-width: 0;
		gap: 0.1rem;
	}
	.small {
		font-size: 0.82rem;
	}
	.note {
		padding: 0.4rem 0.7rem;
		border-radius: var(--radius-sm);
		background: var(--accent-soft);
		margin: 0 0 0.6rem;
	}
	.empty {
		border: 0;
		background: transparent;
		padding: 0.4rem 0;
	}
</style>
