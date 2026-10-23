<script lang="ts">
	// Now playing while this device's player gets going (it can take up to 20 s before the app
	// gives up and uses another player): what is happening, how long it has taken, and after a
	// few seconds a way not to wait.
	import { app } from '#lib/state/app.svelte';
	import { device } from '#lib/player/device.svelte';
	import type { DeviceState } from '#lib/player/device.svelte';

	let {
		/** How long the app waits before using another player (see the layout). */
		limitMs = 20_000,
		/** When to offer a way out. */
		offerAfterMs = 5_000,
	}: { limitMs?: number; offerAfterMs?: number } = $props();

	const STEPS: Record<DeviceState, string> = {
		off: 'Starting this device’s player…',
		starting: 'Starting this device’s player…',
		connected: 'Connected. Waiting for your music server to list this device…',
		reconnecting: 'Reconnecting to your music server…',
		stopped: 'This device’s player has stopped.',
		error: 'This device’s player couldn’t start.',
	};

	const started = Date.now();
	let now = $state(Date.now());
	$effect(() => {
		const t = setInterval(() => (now = Date.now()), 500);
		return () => clearInterval(t);
	});
	const waited = $derived(Math.max(0, now - started));
	const slow = $derived(waited >= offerAfterMs);
	const step = $derived(STEPS[device.state] ?? STEPS.starting);

	async function retry() {
		device.stop();
		await device.start();
	}
</script>

<div class="wait" data-device-wait>
	<h1>Getting this device ready…</h1>
	<p class="muted" role="status" data-device-step>{step}{slow ? ' This is taking longer than usual.' : ''}</p>
	<div
		class="track"
		role="progressbar"
		aria-label="Waiting for this device"
		aria-valuemin="0"
		aria-valuemax={Math.round(limitMs / 1000)}
		aria-valuenow={Math.min(Math.round(limitMs / 1000), Math.round(waited / 1000))}
		aria-valuetext={`${Math.round(waited / 1000)} seconds`}
	>
		<span class="fill" style:width={`${Math.min(100, (waited / limitMs) * 100)}%`}></span>
	</div>
	{#if slow}
		<div class="actions">
			<button class="btn primary" onclick={() => app.deviceFailed()} data-device-other>Use another player</button>
			<button class="btn" onclick={retry} data-device-retry>Try again</button>
		</div>
	{:else}
		<p class="muted small">Music will play here. Pick something to play, or choose another player with the Play on… button.</p>
	{/if}
</div>

<style>
	.wait {
		display: grid;
		gap: 0.5rem;
	}
	h1 {
		margin: 0;
	}
	p {
		margin: 0;
	}
	.small {
		font-size: 0.9rem;
	}
	.track {
		height: 4px;
		border-radius: 2px;
		background: var(--line);
		overflow: hidden;
	}
	.fill {
		display: block;
		height: 100%;
		background: var(--accent);
		transition: width 0.5s linear;
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem;
	}
</style>
