<script lang="ts">
	// The strip shown only while playback is moving to another player (Wave 3.5, rule 1). Which
	// player is in use is shown in the mini player ("on Lounge"); moving playback, and going
	// back to this device, live in Play on….
	import { app } from '$lib/state/app.svelte';
	import Icon from './Icon.svelte';

	let { compact = false }: { compact?: boolean } = $props();
</script>

{#if app.moving}
	<div class="strip" class:compact role="status" data-strip>
		<Icon name="transfer" size={16} /><span class="text">Moving playback to {app.moving}…</span>
	</div>
{/if}

<style>
	.strip {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		min-height: var(--strip-h);
		padding: 0.35rem 0.85rem;
		border-radius: calc(var(--strip-h) / 2);
		background: var(--accent);
		color: var(--accent-fg);
		font-size: 0.85rem;
	}
	.strip.compact {
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
</style>
