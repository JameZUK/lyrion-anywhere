<script lang="ts">
	// The strip shown while another player than this device is in use (SPEC D20): it says so,
	// and offers one tap back to this device, or to bring the playback here (Transfer).
	import { app } from '$lib/state/app.svelte';
	import { playerLabel } from '$lib/lms/sync';
	import { device } from '$lib/player/device.svelte';
	import { bringHere, useThisDevice } from '$lib/player/outputs';
	import Icon from './Icon.svelte';
	import { controls } from '$lib/player/controls.svelte';

	let { compact = false }: { compact?: boolean } = $props();
	const name = $derived(playerLabel(app.playerId, app.now, app.players) || 'another player');
	const hasQueue = $derived(Number(app.now?.playlist_tracks ?? 0) > 0);
</script>

{#if app.moving}
	<div class="strip" class:compact role="status" data-strip>
		<Icon name="transfer" size={16} /><span class="text">Moving playback to {app.moving}…</span>
	</div>
{:else if app.remote}
	<div class="strip" class:compact role="status" data-strip>
		<Icon name="output" size={16} />
		<span class="text"><span class="on-label">Playing on</span> <strong>{name}</strong></span>
		{#if device.supported}
			<!-- Both need the server (moving the queue; starting this device's player). -->
			{#if hasQueue}
				<button class="link-btn" onclick={() => void bringHere()} aria-label="Bring playback to this device" {...controls.needsServer()} data-bring-here>Bring here</button>
			{/if}
			<button class="link-btn" onclick={useThisDevice} aria-label="Back to this device" {...controls.needsServer()} data-this-device><span class="long">Back to this device</span><span class="short">This device</span></button>
		{/if}
	</div>
{/if}

<style>
	.strip {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.35rem 0.5rem;
		min-height: var(--strip-h);
		padding: 0.35rem 0.5rem 0.35rem 0.85rem;
		border-radius: calc(var(--strip-h) / 2);
		background: var(--accent);
		color: var(--accent-fg);
		font-size: 0.85rem;
	}
	.strip.compact {
		flex-wrap: nowrap;
		border-radius: 0;
		padding: 0.2rem 0.75rem;
	}
	.text {
		flex: 1 1 6rem;
		min-width: 0;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.link-btn {
		flex: none;
		border: 1px solid color-mix(in srgb, var(--accent-fg) 55%, transparent);
		background: transparent;
		color: inherit;
		font: inherit;
		font-weight: 600;
		border-radius: 999px;
		padding: 0.2em 0.75em;
		cursor: pointer;
		white-space: nowrap;
	}
	.link-btn:disabled {
		opacity: 0.5;
		cursor: default;
	}
	.link-btn:hover:not(:disabled) {
		background: color-mix(in srgb, var(--accent-fg) 15%, transparent);
	}
	.short {
		display: none;
	}
	/* Phones: the shorter label, so the player's name keeps its room. */
	@media (max-width: 480px) {
		.long {
			display: none;
		}
		.short {
			display: inline;
		}
		.strip {
			flex-wrap: nowrap;
			font-size: 0.8rem;
		}
		/* The player's name matters most: "Playing on" is left to the icon (and read out). */
		.on-label {
			position: absolute;
			width: 1px;
			height: 1px;
			overflow: hidden;
			clip: rect(0 0 0 0);
			white-space: nowrap;
		}
	}
	/* Touch: the small pills still take a 44 px tall tap. */
	@media (pointer: coarse) {
		.link-btn {
			position: relative;
		}
		.link-btn::after {
			content: '';
			position: absolute;
			inset: -8px -2px;
		}
	}
</style>
