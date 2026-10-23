<script lang="ts">
	// The one status line at the top of every page (Wave 3.5, rule 1; see state/status.ts for
	// what it shows and in which order). It takes its own room, never over the controls. The
	// connection message keeps its id: controls disabled by the connection point at it.
	import { app } from '#lib/state/app.svelte';
	import { device } from '#lib/player/device.svelte';
	import { controls } from '#lib/player/controls.svelte';
	import { CONNECTION_ID } from '#lib/state/reachability';
	import { statusOf } from '#lib/state/status';
	import { appUpdate } from '#lib/pwa/update.svelte';
	import Icon from './Icon.svelte';

	const status = $derived(
		statusOf({
			moving: app.moving,
			reach: app.reach,
			message: controls.message,
			reconnecting: controls.thisDevice && device.state === 'reconnecting',
			heldBack: device.needsTap && (device.detail.output === 'Playing' || (app.playerId === device.mac && app.playing)),
			resume: device.wasActive && (device.state === 'off' || device.state === 'stopped' || device.state === 'error'),
			update: appUpdate.offered,
		}),
	);
	const ICONS = { moving: 'transfer', unreachable: 'offline', busy: 'clock', reconnecting: 'clock', held: 'play', resume: 'device', update: 'new' } as const;
</script>

{#if status}
	<div
		class="slot"
		class:accent={status.kind === 'moving' || status.kind === 'busy'}
		role="status"
		id={status.kind === 'unreachable' || status.kind === 'busy' ? CONNECTION_ID : undefined}
		data-status={status.kind}
		data-connection={status.kind === 'unreachable' || status.kind === 'busy' ? app.reach : undefined}
	>
		<Icon name={ICONS[status.kind]} size={16} /><span class="text">{status.text}</span>
		{#if status.kind === 'resume'}
			<button class="btn primary small" onclick={() => device.start()}><Icon name="play" size={16} />Resume playing here</button>
			<button class="btn small" onclick={() => device.stop()}>Dismiss</button>
		{:else if status.kind === 'update'}
			<button class="btn primary small" data-update-reload onclick={() => appUpdate.apply()}>Reload</button>
			<button class="btn small" data-update-later onclick={() => appUpdate.dismiss()}>Later</button>
		{/if}
	</div>
{/if}

<style>
	.slot {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.3rem 0.5rem;
		width: fit-content;
		max-width: calc(100% - 2 * clamp(1rem, 3vw, 2.5rem));
		margin: 0.6rem clamp(1rem, 3vw, 2.5rem) 0;
		padding: 0.3rem 0.75rem;
		border-radius: 999px;
		font-size: 0.85rem;
		background: var(--bg-sunken);
		color: var(--fg);
		border: 1px solid var(--line);
		flex: none;
	}
	.slot.accent {
		border-color: var(--accent);
		background: var(--accent-soft);
	}
	.text {
		min-width: 0;
	}
	.small {
		font-size: 0.85rem;
		padding: 0.25em 0.8em;
	}
	/* Offers with buttons: a rounded box rather than a pill once they wrap. */
	.slot:has(.btn) {
		border-radius: var(--radius-sm);
	}
</style>
