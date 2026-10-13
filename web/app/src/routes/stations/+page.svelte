<script lang="ts">
	import BackButton from '$lib/components/BackButton.svelte';
	// Stations: endless mixes from the library, refilled as they play.
	import { me } from '$lib/state/me';
	import type { Station } from '$lib/api/me';
	import { activeStation } from '$lib/stations/active.svelte';
	import StationCards from '$lib/components/StationCards.svelte';
	import NewStation from '$lib/components/NewStation.svelte';
	import Icon from '$lib/components/Icon.svelte';

	let stations = $state<Station[] | null>(null);
	let error = $state('');
	let making = $state(false);

	async function load() {
		try {
			stations = await me.stations();
		} catch (e) {
			error = (e as Error).message;
		}
	}
	void load();
</script>

<section class="page">
	<BackButton parent="/home" />
	<header>
		<h1>Stations</h1>
		{#if !making}<button class="btn" onclick={() => (making = true)}><Icon name="plus" size={18} />New station</button>{/if}
	</header>
	<p class="muted lead">Endless mixes from your library, topped up as they play. Save a mix from any menu (for example a plugin’s “mix like this”) with Save as station.</p>
	{#if activeStation.playing && activeStation.station}
		<p class="playing" role="status">
			Playing {activeStation.station.name}
			<button class="btn small" onclick={() => activeStation.stop()}>Stop topping up</button>
		</p>
	{/if}
	{#if making}
		<NewStation
			oncreated={(s) => {
				stations = [...(stations ?? []), s];
				making = false;
			}}
			oncancel={() => (making = false)}
		/>
	{/if}
	{#if error}
		<p class="error">{error}</p>
	{:else if stations === null}
		<p class="muted">Loading…</p>
	{:else}
		<StationCards {stations} removable onremoved={(id) => (stations = (stations ?? []).filter((s) => s.id !== id))} />
	{/if}
</section>

<style>
	header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		flex-wrap: wrap;
		gap: 0.5rem 1rem;
	}
	h1 {
		margin: 0;
	}
	.lead {
		margin: 0.25rem 0 1rem;
	}
	.playing {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		padding: 0.5rem 0.75rem;
		border-radius: var(--radius-sm);
		background: var(--accent-soft);
	}
	.small {
		padding: 0.3em 0.7em;
		font-size: 0.85rem;
	}
</style>
