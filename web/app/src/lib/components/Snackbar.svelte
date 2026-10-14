<script lang="ts">
	// A short message above the player bar, with an optional action (e.g. Undo) and, for an
	// offer, a second one ("Other player…"). It goes away on its own after `timeout` ms (0: an
	// offer stays until it is answered or dismissed); the parent clears it then (ondismiss).
	//
	// Wave 3.5 (rule 1): results are snackbars, and only one shows at a time. Every Snackbar
	// joins one list (./snackbars); only the newest message is shown, and an older one comes
	// back when that goes. Keep it mounted with an empty message: its live region (role=status,
	// polite) has to be in the page before a message arrives, or screen readers may not
	// announce it.
	import { snackbars } from './snackbars.svelte';

	let {
		message,
		action = '',
		onaction,
		secondary = '',
		onsecondary,
		ondismiss,
		timeout = 6000,
	}: {
		message: string;
		action?: string;
		onaction?: () => void;
		secondary?: string;
		onsecondary?: () => void;
		ondismiss: () => void;
		timeout?: number;
	} = $props();

	let actionBtn = $state<HTMLButtonElement>();
	const me = snackbars.join();
	$effect(() => () => snackbars.leave(me));
	// A new message comes to the top, even over one that arrived before it.
	$effect(() => {
		const m = message;
		snackbars.set(me, false);
		if (m) snackbars.set(me, true);
	});
	/** Shown: it has a message, and no newer snackbar does. */
	const shown = $derived(!!message && snackbars.top === me);

	$effect(() => {
		if (!message || !timeout) return;
		const t = setTimeout(() => ondismiss(), timeout);
		return () => clearTimeout(t);
	});

	/** Put focus on the action (e.g. after the control that was used has gone). */
	export function focusAction(): boolean {
		actionBtn?.focus();
		return !!actionBtn;
	}
</script>

<div class="snack" class:empty={!shown} data-snackbar data-shown={shown ? 'true' : undefined}>
	<span class="msg" role="status" aria-live="polite">{message}</span>
	{#if shown}
		{#if action && onaction}<button class="act" bind:this={actionBtn} onclick={onaction}>{action}</button>{/if}
		{#if secondary && onsecondary}<button class="act" onclick={onsecondary}>{secondary}</button>{/if}
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
		flex-wrap: wrap;
		align-items: center;
		justify-content: flex-end;
		gap: 0 0.25rem;
		width: min(calc(100vw - 2rem), 460px);
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
		flex: 1 1 12rem;
		min-width: 0;
		padding: 0.3rem 0;
		overflow-wrap: anywhere;
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
