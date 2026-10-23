<script lang="ts">
	// Shown while the music server can't be reached and nobody is signed in yet: it keeps
	// trying, and meanwhile offers this device's downloads (they are local, so they need no
	// sign-in). Only the account last signed in here is offered: its session is still on this
	// device. Signing out forgets it, so nobody else's downloads are ever shown.
	import { downloads, offlinePlayer, plays } from '#lib/offline/index.svelte';
	import { lastOwner } from '#lib/offline/settings';
	import DownloadsView from './DownloadsView.svelte';
	import OfflineBar from './OfflineBar.svelte';
	import Icon from './Icon.svelte';

	let browsing = $state(false);
	const owner = lastOwner();

	$effect(() => {
		if (owner) {
			void downloads.setUser(owner, false);
			void plays.setUser(owner, false);
		}
	});
	const count = $derived(owner && downloads.user === owner ? downloads.saved.length : 0);
</script>

{#if browsing && owner}
	<div class="offline">
		<header class="top">
			<span class="muted"><Icon name="offline" size={18} /> Can’t reach your music server. Trying again…</span>
			<button class="btn" onclick={() => (browsing = false)}>Back</button>
		</header>
		<main class="content"><DownloadsView offline /></main>
		{#if offlinePlayer.active}<OfflineBar href={null} />{/if}
	</div>
{:else}
	<div class="splash" role="status">
		<div class="box">
			<p class="muted">Can’t reach your music server. Trying again…</p>
			{#if count > 0}
				<button class="btn primary" onclick={() => (browsing = true)}><Icon name="play" size={18} />Play your downloads</button>
				<p class="muted small">{count} downloaded {count === 1 ? 'track' : 'tracks'} on this device.</p>
			{/if}
		</div>
	</div>
{/if}

<style>
	.splash {
		min-height: var(--app-h);
		display: grid;
		place-items: center;
		padding: 1rem;
	}
	.box {
		display: grid;
		justify-items: center;
		gap: 0.75rem;
		text-align: center;
	}
	.small {
		font-size: 0.85rem;
		margin: 0;
	}
	.offline {
		--tabs-h: 0px;
		min-height: var(--app-h);
	}
	.top {
		display: flex;
		flex-wrap: wrap;
		gap: 0.6rem;
		align-items: center;
		justify-content: space-between;
		padding: 0.75rem clamp(1rem, 3vw, 2.5rem);
		border-bottom: 1px solid var(--line);
	}
	.top span {
		display: flex;
		align-items: center;
		gap: 0.4rem;
	}
	.content {
		padding-bottom: calc(var(--bar-h) + 1rem);
	}
</style>
