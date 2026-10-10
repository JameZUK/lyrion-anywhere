<script lang="ts">
	// Pick which player to control; start or stop this browser as a player.
	import { app } from '$lib/state/app.svelte';
	import { device } from '$lib/player/device.svelte';
	import { textOf } from '$lib/lms/format';
	import Icon from './Icon.svelte';

	let { onclose }: { onclose: () => void } = $props();

	function pick(id: string) {
		app.select(id);
		onclose();
	}

	async function startHere() {
		await device.start();
	}

	const deviceLabel = $derived(
		{
			off: 'Play on this device',
			starting: 'Starting…',
			connected: 'Playing here',
			reconnecting: 'Reconnecting…',
			stopped: 'Play on this device',
			error: 'Could not start; try again',
		}[device.state],
	);
</script>

<div class="sheet" role="dialog" aria-label="Choose a player">
	<header>
		<h2>Players</h2>
		<button class="icon-btn" onclick={onclose} aria-label="Close"><Icon name="close" /></button>
	</header>

	<div class="here">
		{#if device.state === 'connected' || device.state === 'reconnecting' || device.state === 'starting'}
			<div class="row active-device">
				<Icon name="device" />
				<span class="grow">{deviceLabel}</span>
				<button class="btn" onclick={() => device.stop()}>Stop</button>
			</div>
		{:else if device.supported}
			<button class="row choice" onclick={startHere}>
				<Icon name="device" />
				<span class="grow">{deviceLabel}</span>
				<Icon name="plus" size={18} />
			</button>
		{:else}
			<p class="muted">This browser can't play here (no AudioWorklet).</p>
		{/if}
	</div>

	<ul>
		{#each app.players as p (p.playerid)}
			<li>
				<button class="row choice" class:selected={p.playerid === app.playerId} onclick={() => pick(p.playerid)}>
					<Icon name={p.playerid === device.mac ? 'device' : 'speaker'} />
					<span class="grow">
						<span class="name ellipsis">{textOf(p.name)}</span>
						<span class="sub muted">
							{p.isplaying ? 'Playing' : p.power ? 'On' : 'Off'}{p.playerid === device.mac ? ' · this device' : ''}
						</span>
					</span>
					{#if p.isplaying}<span class="eq" aria-hidden="true"><i></i><i></i><i></i></span>{/if}
				</button>
			</li>
		{:else}
			<li class="muted empty">No players you can control yet.</li>
		{/each}
	</ul>
</div>

<style>
	.sheet {
		background: var(--bg-raised);
		border: 1px solid var(--line);
		border-radius: var(--radius);
		box-shadow: var(--shadow);
		width: min(92vw, 360px);
		max-height: min(70vh, 560px);
		overflow: auto;
		padding: 0.5rem;
	}
	header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 0.25rem 0.25rem 0.25rem 0.75rem;
	}
	h2 {
		margin: 0;
		font-size: 1rem;
	}
	ul {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	.row {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		width: 100%;
		padding: 0.6rem 0.75rem;
		border-radius: var(--radius-sm);
		text-align: left;
	}
	.choice {
		border: 0;
		background: transparent;
	}
	.choice:hover {
		background: var(--bg-sunken);
	}
	.selected {
		background: var(--accent-soft);
	}
	.active-device {
		background: var(--accent-soft);
	}
	.here {
		border-bottom: 1px solid var(--line);
		padding-bottom: 0.4rem;
		margin-bottom: 0.4rem;
	}
	.here p {
		padding: 0.5rem 0.75rem;
		margin: 0;
	}
	.grow {
		flex: 1;
		min-width: 0;
		display: grid;
	}
	.name {
		font-weight: 550;
	}
	.sub {
		font-size: 0.82rem;
	}
	.empty {
		padding: 0.75rem;
	}
	.eq {
		display: inline-flex;
		gap: 2px;
		align-items: flex-end;
		height: 14px;
	}
	.eq i {
		width: 3px;
		background: var(--accent);
		animation: eq 0.9s ease-in-out infinite;
	}
	.eq i:nth-child(2) {
		animation-delay: -0.3s;
	}
	.eq i:nth-child(3) {
		animation-delay: -0.6s;
	}
	@keyframes eq {
		0%,
		100% {
			height: 30%;
		}
		50% {
			height: 100%;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.eq i {
			animation: none;
			height: 70%;
		}
	}
</style>
