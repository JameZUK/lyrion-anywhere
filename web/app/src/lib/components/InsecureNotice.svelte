<script lang="ts">
	// "Unencrypted connection": shown at the top of every page (the sign-in page too) when the
	// app is open on plain http:// to anything but loopback, in the status slot's style. See
	// pwa/insecure.svelte.ts for when, and why.
	import { insecureNotice, type InsecureNotice } from '#lib/pwa/insecure.svelte';
	import Icon from './Icon.svelte';

	let { notice = insecureNotice }: { notice?: InsecureNotice } = $props();
</script>

{#if notice.shown}
	<div class="slot" role="status" data-insecure-notice>
		<Icon name="warning" size={16} /><span class="text">{notice.text}</span>
		<button class="btn small" type="button" data-insecure-dismiss onclick={() => notice.dismiss()}>Dismiss</button>
	</div>
{/if}

<style>
	/* The status slot's look (StatusSlot.svelte), as a rounded box since it holds a button. */
	.slot {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.3rem 0.5rem;
		width: fit-content;
		max-width: calc(100% - 2 * clamp(1rem, 3vw, 2.5rem));
		margin: 0.6rem clamp(1rem, 3vw, 2.5rem) 0;
		padding: 0.3rem 0.75rem;
		border-radius: var(--radius-sm);
		font-size: 0.85rem;
		background: var(--bg-sunken);
		color: var(--fg);
		border: 1px solid var(--line);
		flex: none;
	}
	.text {
		min-width: 0;
	}
	.small {
		font-size: 0.85rem;
		padding: 0.25em 0.8em;
	}
</style>
