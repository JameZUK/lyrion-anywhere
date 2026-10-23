<script lang="ts">
	// The signal path (source, decoding, volume and output), for the sheet opened from the
	// quality badge and for Now playing's Info layout.
	import { app } from '#lib/state/app.svelte';
	import { device } from '#lib/player/device.svelte';
	import { quality } from '#lib/lms/quality';
	import { signalPath } from '#lib/player/signal-path';
	import { streamOf } from '#lib/player/stream-quality';

	const here = $derived(!!app.playerId && app.playerId === device.mac && device.state === 'connected');
	const view = $derived(
		signalPath({
			quality: quality(app.now?.playlist_loop?.[0]),
			thisDevice: here,
			playerName: app.nameOf(app.playerId),
			mode: here ? device.choice : null,
			volumeFollows: here && device.volumePlan ? device.volumePlan.follow : null,
			gain: device.gain,
			engineRate: here && Number(device.detail.sample_rate) > 0 ? Number(device.detail.sample_rate) : null,
			deviceRate: here ? device.deviceRate() : null,
			stream: here ? streamOf(device.detail.source) : null,
			level: here ? device.quality : null,
		}),
	);
</script>

<div class="path" data-signal-path>
	<p class="verdict" class:good={view.verdict === 'lossless'}>{view.headline}</p>
	<dl>
		{#each view.rows as r (r.label)}
			<div class="row">
				<dt>{r.label}</dt>
				<dd>{r.value}</dd>
			</div>
		{/each}
	</dl>
	{#each view.notes as n (n)}<p class="muted note">{n}</p>{/each}
</div>

<style>
	.path {
		display: grid;
		gap: 0.6rem;
	}
	.verdict {
		justify-self: start;
		margin: 0;
		padding: 0.25rem 0.7rem;
		border-radius: 999px;
		background: var(--bg-sunken);
		font-weight: 600;
		font-size: 0.9rem;
	}
	.verdict.good {
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
	}
	dl {
		margin: 0;
		display: grid;
		gap: 0.5rem;
	}
	.row {
		display: grid;
		gap: 0.1rem;
	}
	dt {
		font-size: 0.8rem;
		font-weight: 650;
		color: var(--fg-muted);
	}
	dd {
		margin: 0;
	}
	.note {
		margin: 0;
		font-size: 0.85rem;
	}
</style>
