<script lang="ts">
	// "More…" for a track, album or artist: LMS's info menu, including entries plugins add.
	import * as cmd from '$lib/lms/commands';
	import MenuView from './MenuView.svelte';

	let { kind, id, title, onclose }: { kind: cmd.InfoKind; id: number; title: string; onclose: () => void } = $props();
	let dialog = $state<HTMLDialogElement>();
	$effect(() => {
		dialog?.showModal();
	});
</script>

<dialog bind:this={dialog} onclose={onclose} onclick={(e) => e.target === dialog && dialog?.close()} aria-label={`More for ${title}`}>
	<div class="sheet">
		<MenuView start={cmd.infoMenu(kind, id)} {title} compact onclose={() => dialog?.close()} />
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
