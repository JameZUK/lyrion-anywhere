<script lang="ts">
	// An in-app confirmation (instead of the browser's confirm(), which installed web apps show
	// badly or not at all). A modal <dialog>: focus starts on Cancel, Escape or a tap outside
	// cancels, and focus goes back to what opened it.
	let {
		title,
		message = '',
		confirmLabel = 'OK',
		cancelLabel = 'Cancel',
		danger = false,
		onconfirm,
		oncancel,
	}: {
		title: string;
		message?: string;
		confirmLabel?: string;
		cancelLabel?: string;
		danger?: boolean;
		onconfirm: () => void;
		oncancel: () => void;
	} = $props();

	let dialog = $state<HTMLDialogElement>();
	let cancelBtn = $state<HTMLButtonElement>();
	const id = `confirm-${Math.random().toString(36).slice(2, 9)}`;
	/** What had focus before: focus goes back there whichever way the dialog closes. */
	const opener = typeof document !== 'undefined' ? (document.activeElement as HTMLElement | null) : null;
	let answered = false;

	$effect(() => {
		const d = dialog;
		if (d && !d.open) d.showModal?.();
		cancelBtn?.focus();
	});

	function finish(yes: boolean) {
		if (answered) return;
		answered = true;
		if (dialog?.open) dialog.close();
		if (opener?.isConnected) opener.focus();
		if (yes) onconfirm();
		else oncancel();
	}
</script>

<dialog
	bind:this={dialog}
	role="alertdialog"
	aria-modal="true"
	aria-labelledby={`${id}-t`}
	aria-describedby={message ? `${id}-m` : undefined}
	onclose={() => finish(false)}
	oncancel={(e) => {
		e.preventDefault();
		finish(false);
	}}
	onclick={(e) => e.target === dialog && finish(false)}
	data-confirm-dialog
>
	<div class="box">
		<h2 id={`${id}-t`}>{title}</h2>
		{#if message}<p id={`${id}-m`}>{message}</p>{/if}
		<div class="actions">
			<button class="btn" bind:this={cancelBtn} onclick={() => finish(false)} data-confirm-cancel>{cancelLabel}</button>
			<button class="btn primary" class:danger onclick={() => finish(true)} data-confirm-ok>{confirmLabel}</button>
		</div>
	</div>
</dialog>

<style>
	dialog {
		border: 0;
		padding: 0;
		background: transparent;
		color: var(--fg);
		max-width: min(92vw, 400px);
		width: 100%;
	}
	dialog::backdrop {
		background: rgb(0 0 0 / 45%);
	}
	.box {
		background: var(--bg-raised);
		color: var(--fg);
		border-radius: var(--radius);
		padding: 1.1rem 1.1rem 1rem;
		box-shadow: var(--shadow);
	}
	h2 {
		margin: 0 0 0.5rem;
		font-size: 1.1rem;
	}
	p {
		margin: 0 0 1rem;
		color: var(--fg-muted);
	}
	.actions {
		display: flex;
		justify-content: flex-end;
		flex-wrap: wrap;
		gap: 0.5rem;
	}
	.actions .btn {
		min-height: 44px;
	}
	.danger {
		background: var(--danger, #b3261e);
		border-color: var(--danger, #b3261e);
		color: #fff;
	}
</style>
