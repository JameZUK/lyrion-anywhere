<script lang="ts">
	// A short message above the player bar, with an optional action (e.g. Undo). It goes away
	// on its own after `timeout` ms; the parent removes it then (ondismiss).
	let {
		message,
		action = '',
		onaction,
		ondismiss,
		timeout = 6000,
	}: { message: string; action?: string; onaction?: () => void; ondismiss: () => void; timeout?: number } = $props();

	$effect(() => {
		void message; // a new message restarts the clock
		const t = setTimeout(() => ondismiss(), timeout);
		return () => clearTimeout(t);
	});
</script>

<div class="snack" role="status" aria-live="polite">
	<span class="msg">{message}</span>
	{#if action && onaction}<button class="act" onclick={onaction}>{action}</button>{/if}
	<button class="icon-btn" aria-label="Dismiss" onclick={ondismiss}>
		<svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" /></svg>
	</button>
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
		min-height: 40px;
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
