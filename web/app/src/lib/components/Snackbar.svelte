<script lang="ts">
	// A short message above the player bar, with an optional action (e.g. Undo). It goes away
	// on its own after `timeout` ms; the parent clears it then (ondismiss).
	//
	// Keep it mounted with an empty message: its live region (role=status, polite) has to be in
	// the page before a message arrives, or screen readers may not announce it.
	let {
		message,
		action = '',
		onaction,
		ondismiss,
		timeout = 6000,
	}: { message: string; action?: string; onaction?: () => void; ondismiss: () => void; timeout?: number } = $props();

	let actionBtn = $state<HTMLButtonElement>();

	$effect(() => {
		if (!message) return;
		const t = setTimeout(() => ondismiss(), timeout);
		return () => clearTimeout(t);
	});

	/** Put focus on the action (e.g. after the control that was used has gone). */
	export function focusAction(): boolean {
		actionBtn?.focus();
		return !!actionBtn;
	}
</script>

<div class="snack" class:empty={!message} data-snackbar>
	<span class="msg" role="status" aria-live="polite">{message}</span>
	{#if message}
		{#if action && onaction}<button class="act" bind:this={actionBtn} onclick={onaction}>{action}</button>{/if}
		<button class="icon-btn" aria-label="Dismiss" onclick={ondismiss}>
			<svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" /></svg>
		</button>
	{/if}
</div>

<style>
	.snack {
		position: fixed;
		left: 50%;
		transform: translateX(-50%);
		bottom: calc(var(--tabs-h) + var(--bar-h) + 0.75rem);
		z-index: 30;
		display: flex;
		align-items: center;
		gap: 0.5rem;
		width: min(calc(100vw - 2rem), 420px);
		padding: 0.35rem 0.35rem 0.35rem 1rem;
		border-radius: var(--radius-sm);
		background: var(--fg);
		color: var(--bg);
		box-shadow: var(--shadow);
	}
	/* No message: nothing to see or touch, but the live region stays in the page. */
	.snack.empty {
		width: 1px;
		height: 1px;
		padding: 0;
		overflow: hidden;
		clip: rect(0 0 0 0);
		background: none;
		box-shadow: none;
		pointer-events: none;
	}
	.msg {
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.act {
		border: 0;
		background: transparent;
		color: var(--accent-soft);
		font-weight: 650;
		padding: 0.6rem 0.8rem;
		min-height: 44px;
		border-radius: var(--radius-sm);
	}
	.icon-btn {
		color: inherit;
	}
	.icon-btn:hover,
	.act:hover {
		background: color-mix(in srgb, var(--bg) 15%, transparent);
	}
</style>
