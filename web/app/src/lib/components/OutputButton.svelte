<script lang="ts">
	// The "Play on…" button (SPEC D20). It opens the list of places the music can play; it is
	// marked while another player than this device is in use. `named`: the player's name beside
	// the icon (desktop Now playing, where no mini player says it). `volume`: the sheet carries
	// the volume slider (phones, whose Now playing has no volume row).
	import { app } from '#lib/state/app.svelte';
	import { isThisDevice } from '#lib/player/outputs';
	import Icon from './Icon.svelte';
	import PlayerSwitcher from './PlayerSwitcher.svelte';

	let {
		size = 22,
		up = true,
		open = $bindable(false),
		named = false,
		volume = false,
	}: { size?: number; up?: boolean; open?: boolean; named?: boolean; volume?: boolean } = $props();
	const name = $derived(!app.current ? '' : isThisDevice(app.playerId) ? 'This device' : app.nameOf(app.playerId) || 'Another player');
	const label = $derived(app.remote ? `Play on… (now ${app.nameOf(app.playerId) || 'another player'})` : 'Play on…');
</script>

<div class="output">
	<button
		class="icon-btn"
		class:remote={app.remote}
		class:named
		onclick={() => (open = !open)}
		aria-expanded={open}
		aria-haspopup="dialog"
		aria-label={label}
		title="Play on…"
		data-output-button
	>
		<Icon name="output" {size} />{#if named}<span class="name ellipsis">{name || 'Choose a player'}</span>{/if}
	</button>
	{#if open}
		<div class="pop" class:up><PlayerSwitcher onclose={() => (open = false)} {volume} /></div>
	{/if}
</div>

<style>
	.output {
		position: relative;
		flex: none;
		min-width: 0;
	}
	.remote {
		color: var(--accent);
	}
	.named {
		width: auto;
		max-width: 16rem;
		display: inline-flex;
		gap: 0.45rem;
		padding: 0 0.85rem;
		border-radius: 999px;
	}
	.name {
		min-width: 0;
		font-weight: 600;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.pop {
		position: absolute;
		right: 0;
		top: calc(100% + 8px);
		z-index: 30;
	}
	.pop.up {
		top: auto;
		bottom: calc(100% + 8px);
	}
	/* Phones: a sheet across the bottom of the screen, above the tab bar. */
	@media (max-width: 760px) {
		.pop,
		.pop.up {
			position: fixed;
			left: 0.5rem;
			right: 0.5rem;
			top: auto;
			bottom: var(--pop-bottom, calc(var(--tabs-h) + 0.5rem));
			display: grid;
			justify-items: stretch;
		}
		.pop :global(.sheet) {
			width: auto;
			max-height: min(75vh, 640px);
		}
	}
</style>
