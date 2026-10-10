<script lang="ts">
	// Sign out, asking first what to do with this account's downloads. Keeping them is the
	// default (a personal device); removing them is one tap (a shared one).
	import { formatBytes } from '$lib/offline/settings';
	import Icon from './Icon.svelte';

	let {
		count,
		bytes = 0,
		onsignout,
	}: {
		/** How many tracks this account has downloaded here. */
		count: number;
		bytes?: number;
		onsignout: (removeDownloads: boolean) => void | Promise<void>;
	} = $props();

	let asking = $state(false);
	let busy = $state(false);

	async function go(remove: boolean) {
		busy = true;
		try {
			await onsignout(remove);
		} finally {
			busy = false;
			asking = false;
		}
	}
</script>

{#if asking}
	<div class="ask" role="group" aria-labelledby="signout-q">
		<p id="signout-q"><strong>Keep your downloads on this device?</strong></p>
		<p class="muted small">
			You have {count} downloaded {count === 1 ? 'track' : 'tracks'} ({formatBytes(bytes)}) here. Downloads stay on this device until
			you remove them; they are only shown again when you sign in. On a shared device, remove them.
		</p>
		<div class="row">
			<button class="btn primary" disabled={busy} onclick={() => go(false)}><Icon name="power" size={18} />Keep downloads and sign out</button>
			<button class="btn" disabled={busy} onclick={() => go(true)}><Icon name="trash" size={18} />Remove downloads and sign out</button>
			<button class="btn" disabled={busy} onclick={() => (asking = false)}>Cancel</button>
		</div>
	</div>
{:else}
	<button class="btn" disabled={busy} onclick={() => (count > 0 ? (asking = true) : go(false))}><Icon name="power" size={18} />Sign out</button>
{/if}

<style>
	.ask p {
		margin: 0 0 0.5rem;
	}
	.small {
		font-size: 0.85rem;
	}
	.row {
		display: flex;
		flex-wrap: wrap;
		gap: 0.6rem;
	}
</style>
