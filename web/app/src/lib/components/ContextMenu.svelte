<script lang="ts">
	// "More…" for a track, album or artist: LMS's info menu, including entries plugins add. For
	// a track, under the heading and its close button: the full 1–5 rating (Now playing shows a
	// single toggle) and the credits, each name a link to that contributor in that role.
	import * as cmd from '#lib/lms/commands';
	import MenuView from './MenuView.svelte';
	import RatingControl from './RatingControl.svelte';
	import TrackCredits from './TrackCredits.svelte';

	let { kind, id, title, onclose }: { kind: cmd.InfoKind; id: number; title: string; onclose: () => void } = $props();
	let dialog = $state<HTMLDialogElement>();
	$effect(() => {
		dialog?.showModal();
	});
</script>

{#snippet trackIntro()}
	<div class="rate" data-track-rating><span class="muted">Your rating</span><RatingControl track={id} style="stars" /></div>
	<TrackCredits track={id} onfollow={() => dialog?.close()} />
{/snippet}

<dialog bind:this={dialog} onclose={onclose} onclick={(e) => e.target === dialog && dialog?.close()} aria-label={`More for ${title}`}>
	<div class="sheet">
		<MenuView start={cmd.infoMenu(kind, id)} {title} compact onclose={() => dialog?.close()} intro={kind === 'track' ? trackIntro : undefined} />
	</div>
</dialog>

<style>
	dialog {
		border: 0;
		padding: 0;
		background: transparent;
		max-width: min(96vw, 560px);
		width: 100%;
		max-height: 86vh;
	}
	.rate {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.25rem 0.75rem;
		margin: 0 0 0.5rem;
	}
	dialog::backdrop {
		background: rgb(0 0 0 / 45%);
	}
	.sheet {
		background: var(--bg-raised);
		color: var(--fg);
		border-radius: var(--radius);
		padding: 1rem;
		max-height: 86vh;
		overflow: auto;
		box-shadow: var(--shadow);
	}
</style>
