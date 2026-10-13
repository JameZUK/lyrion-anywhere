<script lang="ts">
	// Full-screen lyrics (docs/plans/wave1-design.md §9): the lines (LyricsLines) in a dialog.
	// Escape or Close returns to Now playing.
	import Icon from './Icon.svelte';
	import LyricsLines from './LyricsLines.svelte';

	let { title = '', onclose }: { title?: string; onclose: () => void } = $props();

	let dialog = $state<HTMLDialogElement>();

	$effect(() => {
		const d = dialog;
		if (d && !d.open) d.showModal?.();
	});
</script>

<dialog bind:this={dialog} {onclose} class="lyrics" aria-label={title ? `Lyrics: ${title}` : 'Lyrics'} data-lyrics-view>
	<header>
		<h2>{title || 'Lyrics'}</h2>
		<button class="icon-btn" aria-label="Close lyrics" onclick={() => (dialog?.open ? dialog.close() : onclose())}><Icon name="close" /></button>
	</header>
	<LyricsLines />
</dialog>

<style>
	dialog.lyrics {
		border: 0;
		padding: 0;
		margin: 0;
		width: 100vw;
		height: var(--app-h, 100dvh);
		max-width: none;
		max-height: none;
		background: var(--bg);
		color: var(--fg);
		display: flex;
		flex-direction: column;
	}
	dialog.lyrics:not([open]) {
		display: none;
	}
	dialog::backdrop {
		background: var(--bg);
	}
	header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 0.5rem;
		padding: 0.75rem 1rem;
		border-bottom: 1px solid var(--line);
	}
	h2 {
		margin: 0;
		font-size: 1.05rem;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
</style>
