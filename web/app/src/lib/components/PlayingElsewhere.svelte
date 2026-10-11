<script lang="ts">
	// The strip shown while another player than this device is in use (SPEC D20): it says so,
	// and offers one tap back to this device, or to bring the playback here (Transfer).
	import { app } from '$lib/state/app.svelte';
	import { playerLabel } from '$lib/lms/sync';
	import { device } from '$lib/player/device.svelte';
	import { bringHere, useThisDevice } from '$lib/player/outputs';
	import Icon from './Icon.svelte';

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
		<span class="text">Playing on <strong>{name}</strong></span>
		{#if device.supported}
			{#if hasQueue}
				<button class="link-btn" onclick={() => void bringHere()} aria-label="Bring playback to this device">Bring here</button>
			{/if}
			<button class="link-btn" onclick={useThisDevice} aria-label="Back to this device"><span class="long">Back to this device</span><span class="short">This device</span></button>
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
	.link-btn:hover {
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
			font-size: 0.8rem;
		}
	}
</style>
