<script lang="ts">
	// The small "Play on…" button next to the volume (SPEC D20). It opens the list of places
	// the music can play; it is marked while another player than this device is in use.
	import { app } from '$lib/state/app.svelte';
	import Icon from './Icon.svelte';
	import PlayerSwitcher from './PlayerSwitcher.svelte';

	let { size = 22, up = true }: { size?: number; up?: boolean } = $props();
	let open = $state(false);
	const label = $derived(app.remote ? `Play on… (now ${app.nameOf(app.playerId) || 'another player'})` : 'Play on…');
</script>

<div class="output">
	<button class="icon-btn" class:remote={app.remote} onclick={() => (open = !open)} aria-expanded={open} aria-haspopup="dialog" aria-label={label} title="Play on…" data-output-button>
		<Icon name="output" {size} />
	</button>
	{#if open}
		<div class="pop" class:up><PlayerSwitcher onclose={() => (open = false)} /></div>
	{/if}
</div>

<style>
	.output {
		position: relative;
		flex: none;
	}
	.remote {
		color: var(--accent);
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
