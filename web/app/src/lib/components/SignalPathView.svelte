<script lang="ts">
	// The signal path (docs/plans/signal-path-redesign.md), for the sheet opened from the
	// quality badge and for Now playing's Info layout: the shared model's blocks as cards
	// (File → LMS → Lyrion gateway → This device; File → a remote player; Download → This
	// device), stacked by default and in a row with arrows when `row` is set (the sheet on a wide
	// screen). The block that lowers the verdict is tinted amber. The quality switch (this
	// device only) is under the gateway block when stacked, and on its own full-width line under
	// the row otherwise, so its pills never wrap in a narrow column; under the device block,
	// Details: the stage list, rates and notes as before.
	import { app } from '#lib/state/app.svelte';
	import { device } from '#lib/player/device.svelte';
	import { quality } from '#lib/lms/quality';
	import { signalPath } from '#lib/player/signal-path';
	import { levelName, type Level } from '#lib/player/quality-path';
	import { amber, qualityNow } from '#lib/player/quality-now.svelte';
	import { streamOf } from '#lib/player/stream-quality';

	/** The blocks in a row with the switch beneath (wide screens); stacked otherwise. */
	let { row = false }: { row?: boolean } = $props();

	const diagram = $derived(qualityNow.diagram);
	const here = $derived(qualityNow.here);
	const offline = $derived(qualityNow.facts.player === 'offline');
	const sw = $derived(here ? diagram.switch : null);
	/** The wide layout's columns: a card, an arrow, a card, … */
	const cols = $derived(diagram.blocks.map(() => 'minmax(0, 1fr)').join(' auto '));
	// Details: what today's panel showed (nothing is lost), for an LMS player.
	const details = $derived(
		offline
			? null
			: signalPath({
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
	/**
	 * The stream heard now: the track (its id and place in the queue) and the gateway's stream
	 * number. A new one is the "next track" a choice applied to.
	 */
	const streamKey = $derived(`${app.now?.playlist_loop?.[0]?.id ?? ''}|${app.now?.playlist_cur_index ?? ''}|${here ? (device.detail.stream_seq ?? '') : ''}`);
	/** What the last choice on the switch did (started the new stream now, or from the next track), and on which stream. */
	let applied = $state<{ result: 'now' | 'next' | 'unchanged'; on: string } | null>(null);
	/** "Applies from the next track", until that track has started. */
	const appliesNext = $derived(applied?.result === 'next' && applied.on === streamKey);
	function choose(level: Level | null) {
		applied = { result: device.overrideQuality(level, { seekable: sw?.seekable === true, seconds: app.elapsed }), on: streamKey };
	}
	const detailsAt = $derived(diagram.blocks.findLastIndex((b) => b.kind === 'device' || b.kind === 'player'));
</script>

{#snippet qualitySwitch()}
	{#if sw}
		<div class="switch" role="radiogroup" aria-label="Streaming quality" data-quality-switch>
			{#each sw.levels as level (level)}
				<button
					type="button"
					role="radio"
					aria-checked={sw.in_force === level}
					class:on={sw.in_force === level}
					onclick={() => choose(level)}
					data-level={level}>{levelName(level)}</button
				>
			{/each}
		</div>
		{#if sw.origin_text}<p class="caption" data-origin>{sw.origin_text}</p>{/if}
		{#if !sw.seekable || appliesNext}<p class="caption" data-applies-next>Applies from the next track.</p>{/if}
		{#if sw.overridden}<button type="button" class="link back" onclick={() => choose(null)} data-back-to-automatic>Back to automatic</button>{/if}
	{/if}
{/snippet}

<div class="path" class:row data-signal-path data-layout={row ? 'row' : 'stack'}>
	{#if diagram.badge}<p class="verdict" class:amber={amber(diagram)} data-verdict={diagram.badge.verdict}>{diagram.badge.text}</p>{/if}
	<ol class="blocks" style:--cols={cols} aria-label="The path from the file to what you hear">
		{#each diagram.blocks as b, i (b.kind)}
			{#if i > 0}<li class="arrow" aria-hidden="true">→</li>{/if}
			<li class="block" class:lowers={b.lowers} data-block={b.kind} data-lowers={b.lowers ? '' : undefined}>
				<h3 class="title">{b.title}</h3>
				{#if b.format}<p class="format">{b.format}</p>{/if}
				{#if b.note}<p class="note">{b.note}</p>{/if}
				{#if b.kind === 'gateway' && !row}{@render qualitySwitch()}{/if}
				{#if i === detailsAt && details}
					<details class="details" data-details>
						<summary>Details</summary>
						<dl>
							{#each details.rows as r (r.label)}
								<div class="row">
									<dt>{r.label}</dt>
									<dd>{r.value}</dd>
								</div>
							{/each}
						</dl>
						{#each details.notes as n (n)}<p class="muted small">{n}</p>{/each}
					</details>
				{/if}
			</li>
		{/each}
	</ol>
	{#if row && sw}
		<div class="switch-line" data-switch-line>{@render qualitySwitch()}</div>
	{/if}
</div>

<style>
	.path {
		display: grid;
		gap: 0.75rem;
	}
	.verdict {
		justify-self: start;
		margin: 0;
		padding: 0.25rem 0.7rem;
		border-radius: 999px;
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
		font-weight: 600;
		font-size: 0.9rem;
	}
	.verdict.amber {
		background: var(--amber-soft);
		color: var(--amber-fg);
	}
	/* Stacked: one card under another, arrows pointing down. */
	.blocks {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		gap: 0.25rem;
	}
	.arrow {
		justify-self: center;
		align-self: center;
		color: var(--fg-muted);
		line-height: 1;
		transform: rotate(90deg);
	}
	.block {
		display: grid;
		align-content: start;
		gap: 0.2rem;
		min-width: 0;
		padding: 0.6rem 0.75rem;
		border: 1px solid var(--line);
		border-radius: var(--radius-sm);
		background: var(--bg-raised);
	}
	.block.lowers {
		border-color: var(--amber);
		background: var(--amber-soft);
	}
	.title {
		margin: 0;
		font-size: 0.8rem;
		font-weight: 650;
		color: var(--fg-muted);
		overflow-wrap: anywhere;
	}
	.block.lowers .title {
		color: var(--amber-fg);
	}
	.format {
		margin: 0;
		font-weight: 650;
	}
	.note,
	.caption {
		margin: 0;
		font-size: 0.85rem;
	}
	.caption {
		color: var(--fg-muted);
	}
	/* A row of cards with arrows between (the sheet on a wide screen). */
	.row .blocks {
		grid-template-columns: var(--cols);
		gap: 0.4rem;
	}
	.row .arrow {
		transform: none;
	}
	/* The switch and its caption on their own line beneath the row. */
	.switch-line {
		display: grid;
		gap: 0.2rem;
		justify-items: start;
	}
	.switch-line .switch {
		margin-top: 0;
	}
	.switch {
		display: inline-flex;
		flex-wrap: wrap;
		margin-top: 0.35rem;
		border: 1px solid var(--line);
		border-radius: 999px;
		overflow: hidden;
		justify-self: start;
	}
	.switch button {
		min-height: 36px;
		padding: 0 0.75rem;
		border: 0;
		background: transparent;
		color: var(--fg);
		font: inherit;
		font-size: 0.85rem;
		white-space: nowrap;
		cursor: pointer;
	}
	.switch button + button {
		border-left: 1px solid var(--line);
	}
	.switch button.on {
		background: var(--accent);
		color: var(--accent-fg);
		font-weight: 650;
	}
	.back {
		justify-self: start;
		padding: 0.25rem 0;
		background: none;
		border: 0;
		color: var(--accent);
		font: inherit;
		font-size: 0.85rem;
		text-decoration: underline;
		cursor: pointer;
	}
	.details {
		margin-top: 0.35rem;
	}
	.details summary {
		cursor: pointer;
		font-size: 0.85rem;
		font-weight: 600;
	}
	dl {
		margin: 0.4rem 0 0;
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
	.small {
		margin: 0.4rem 0 0;
		font-size: 0.85rem;
	}
</style>
