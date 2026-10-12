<script lang="ts">
	// A modal sheet: a bottom sheet on phones, a centred panel elsewhere. Escape, the close
	// button or a tap outside closes it; focus goes back to what opened it (native <dialog>).
	import type { Snippet } from 'svelte';
	import Icon from './Icon.svelte';

	let { title, onclose, children, label = title }: { title: string; onclose: () => void; children: Snippet; label?: string } = $props();
	let dialog = $state<HTMLDialogElement>();
	$effect(() => {
		const d = dialog;
		if (d && !d.open) d.showModal?.();
	});
</script>

<dialog bind:this={dialog} {onclose} onclick={(e) => e.target === dialog && dialog?.close()} aria-label={label}>
	<div class="sheet">
		<header>
			<h2>{title}</h2>
			<button class="icon-btn" aria-label="Close" onclick={() => (dialog?.open ? dialog.close() : onclose())}><Icon name="close" /></button>
		</header>
		{@render children()}
	</div>
</dialog>

<style>
	dialog {
		border: 0;
		padding: 0;
		background: transparent;
		color: var(--fg);
		max-width: min(96vw, 460px);
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
		padding: 0.75rem 1rem 1rem;
		max-height: 86vh;
		overflow: auto;
		box-shadow: var(--shadow);
	}
	header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 0.5rem;
		margin-bottom: 0.5rem;
	}
	h2 {
		margin: 0;
		font-size: 1.1rem;
	}
	@media (max-width: 760px) {
		dialog {
			margin: auto 0 0;
			max-width: 100vw;
		}
		.sheet {
			border-radius: var(--radius) var(--radius) 0 0;
			padding-bottom: max(1rem, env(safe-area-inset-bottom));
		}
	}
</style>
