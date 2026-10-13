<script lang="ts">
	// Now playing's Info layout (docs/plans/wave2-design.md §4.2): the quality badge, the
	// signal path behind a "Signal path" disclosure (closed until asked for, like the sheet
	// the badge opens), the track's credits as links, its rating and an "Up next" card.
	import { app } from '$lib/state/app.svelte';
	import { isRemote } from '$lib/lms/format';
	import { badgeText, quality } from '$lib/lms/quality';
	import RatingControl from './RatingControl.svelte';
	import SignalPathView from './SignalPathView.svelte';
	import TrackCredits from './TrackCredits.svelte';
	import UpNext from './UpNext.svelte';

	const track = $derived(app.now?.playlist_loop?.[0]);
	const badge = $derived(quality(track));
	/** The signal path stays folded away until asked for. */
	let pathOpen = $state(false);
	const pathId = `np-info-path-${Math.random().toString(36).slice(2, 8)}`;
	/** A library track (not a stream): it has credits and a rating. */
	const libraryId = $derived(typeof track?.id === 'number' && track.id > 0 && !isRemote(track.remote) ? track.id : null);
</script>

<div class="info-pane" data-info-pane>
	{#if !app.now || (!track && !app.now.current_title)}
		<p class="muted">Nothing playing.</p>
	{:else}
		<section aria-labelledby="np-info-quality">
			<h3 id="np-info-quality">Quality</h3>
			{#if badge}<p class="badge" class:hires={badge.hiRes}>{badgeText(badge)}</p>{/if}
			<button class="disclose" aria-expanded={pathOpen} aria-controls={pathId} onclick={() => (pathOpen = !pathOpen)} data-signal-path-toggle>
				<span class="chev" class:open={pathOpen} aria-hidden="true">›</span>Signal path
			</button>
			<div id={pathId} hidden={!pathOpen}>
				{#if pathOpen}<SignalPathView />{/if}
			</div>
		</section>
		{#if libraryId !== null}
			<section aria-labelledby="np-info-rating">
				<h3 id="np-info-rating">Rating</h3>
				<RatingControl track={libraryId} />
			</section>
			<TrackCredits track={libraryId} />
		{/if}
		<UpNext />
	{/if}
</div>

<style>
	.info-pane {
		display: grid;
		gap: 1rem;
		align-content: start;
		padding: 0.25rem 0.25rem 1rem;
		text-align: left;
	}
	section {
		display: grid;
		gap: 0.4rem;
	}
	h3 {
		margin: 0;
		font-size: 0.8rem;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--fg-muted);
	}
	.badge {
		justify-self: start;
		margin: 0;
		padding: 0.1rem 0.55rem;
		border: 1px solid var(--line);
		border-radius: 999px;
		font-size: 0.78rem;
		font-weight: 650;
		letter-spacing: 0.02em;
	}
	.badge.hires {
		border-color: var(--accent);
	}
	.disclose {
		justify-self: start;
		display: inline-flex;
		align-items: center;
		gap: 0.4rem;
		min-height: 44px;
		padding: 0 0.75rem 0 0.5rem;
		border: 1px solid var(--line);
		border-radius: 999px;
		background: transparent;
		color: var(--fg);
		font: inherit;
		font-size: 0.9rem;
		font-weight: 600;
		cursor: pointer;
	}
	.disclose:hover {
		background: color-mix(in srgb, var(--fg) 8%, transparent);
	}
	.chev {
		display: inline-block;
		font-size: 1.1rem;
		line-height: 1;
		transition: transform 0.15s ease;
	}
	.chev.open {
		transform: rotate(90deg);
	}
	@media (prefers-reduced-motion: reduce) {
		.chev {
			transition: none;
		}
	}
</style>
