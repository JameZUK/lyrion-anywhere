<script lang="ts">
	// Now playing's hero on phones: Cover, Lyrics and Track info side by side, one shown at a
	// time. A sideways swipe (touch, pen or mouse) moves between them, as do ← and → on the
	// pager, and the dots under it (a tab list, with the arrow keys of tabs). A swipe up over
	// the cover peeks at the queue. A swipe across a visualiser drawing over the cover changes
	// its style instead (it handles that itself); over Glow, which draws nothing there, it
	// still moves the pager. A page's content is made the first time it is shown, and the
	// pages not shown are inert.
	import type { Snippet } from 'svelte';
	import { LAYOUTS, type Layout } from '$lib/nowplaying/prefs';
	import { PAGE_LABELS, pageForKey, pagerGesture, stepPage } from '$lib/nowplaying/pager';

	let {
		page,
		onpage,
		onpeek,
		still = false,
		cover,
		lyrics,
		info,
	}: {
		page: Layout;
		onpage: (p: Layout) => void;
		/** A swipe up over the cover. */
		onpeek?: () => void;
		/** No sliding (reduced motion). */
		still?: boolean;
		cover: Snippet;
		lyrics: Snippet;
		info: Snippet;
	} = $props();

	const uid = $props.id();
	const index = $derived(Math.max(0, LAYOUTS.indexOf(page)));
	/** Pages shown at least once: their content stays made (lyrics keep their place). */
	let seen = $state<Record<Layout, boolean>>({ art: false, lyrics: false, info: false });
	$effect(() => {
		if (!seen[page]) seen[page] = true;
	});

	function go(p: Layout) {
		if (p !== page) onpage(p);
	}

	// --- swipes -----------------------------------------------------------------------------
	let start: { id: number; x: number; y: number; type: string } | null = null;
	function down(e: PointerEvent) {
		if (start || (e.pointerType === 'mouse' && e.button !== 0)) return;
		const vis = (e.target as Element | null)?.closest?.('[data-visualiser]');
		if (vis && vis.getAttribute('data-visualiser') !== 'glow') return;
		start = { id: e.pointerId, x: e.clientX, y: e.clientY, type: e.pointerType };
	}
	function up(e: PointerEvent) {
		const s = start;
		if (!s || s.id !== e.pointerId) return;
		start = null;
		const g = pagerGesture(e.clientX - s.x, e.clientY - s.y, s.type);
		if (g === 'next' || g === 'previous') go(stepPage(page, g === 'next' ? 1 : -1));
		else if (g === 'peek' && page === 'art') onpeek?.();
	}

	// --- keys ----------------------------------------------------------------------------------
	function onkeydown(e: KeyboardEvent) {
		if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
		// Only on the pager itself (its content, e.g. a lyric line, keeps its own keys).
		if (e.target !== e.currentTarget) return;
		const to = pageForKey(page, e.key);
		if (!to) return;
		e.preventDefault();
		go(to);
	}
	let tabs = $state<HTMLElement>();
	function tabKey(e: KeyboardEvent) {
		if (e.altKey || e.ctrlKey || e.metaKey) return;
		const to = pageForKey(page, e.key);
		if (!to) return;
		e.preventDefault();
		go(to);
		tabs?.querySelector<HTMLButtonElement>(`[data-page-tab="${to}"]`)?.focus();
	}
</script>

<div class="pager-wrap">
	<!-- A carousel: the region itself takes focus so ← and → move it (as the dots' tabs do). -->
	<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
	<div
		class="pager"
		role="region"
		aria-roledescription="carousel"
		aria-label="Cover, lyrics and track info. Use the arrow keys to move between them."
		tabindex="0"
		{onkeydown}
		onpointerdown={down}
		onpointerup={up}
		onpointercancel={() => (start = null)}
		data-pager
		data-page={page}
	>
		<div class="track" class:still style:transform={`translateX(${-index * 100}%)`}>
			{#each LAYOUTS as l (l)}
				<div
					class="panel"
					class:current={l === page}
					role="tabpanel"
					id={`${uid}-panel-${l}`}
					aria-labelledby={`${uid}-tab-${l}`}
					aria-hidden={l === page ? undefined : 'true'}
					inert={l !== page}
					data-page-panel={l}
				>
					{#if seen[l] || l === page}
						{#if l === 'art'}{@render cover()}{:else if l === 'lyrics'}{@render lyrics()}{:else}{@render info()}{/if}
					{/if}
				</div>
			{/each}
		</div>
	</div>
	<div class="dots" role="tablist" aria-label="Show" bind:this={tabs} data-page-dots>
		{#each LAYOUTS as l (l)}
			<button
				role="tab"
				id={`${uid}-tab-${l}`}
				aria-selected={l === page}
				aria-controls={`${uid}-panel-${l}`}
				aria-label={PAGE_LABELS[l]}
				tabindex={l === page ? 0 : -1}
				onclick={() => go(l)}
				onkeydown={tabKey}
				data-page-tab={l}
			><span aria-hidden="true"></span></button>
		{/each}
	</div>
</div>

<style>
	.pager-wrap {
		flex: 1 1 0;
		min-height: 0;
		width: 100%;
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 2px;
	}
	.pager {
		position: relative;
		flex: 1 1 0;
		min-height: 0;
		width: 100%;
		overflow: hidden;
		border-radius: var(--radius);
		/* Sideways swipes move the pager; up and down scroll lyrics and info. */
		touch-action: pan-y pinch-zoom;
	}
	.pager:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: 2px;
	}
	.track {
		display: flex;
		height: 100%;
		transition: transform 280ms cubic-bezier(0.2, 0.8, 0.2, 1);
	}
	.track.still {
		transition: none;
	}
	@media (prefers-reduced-motion: reduce) {
		.track {
			transition: none;
		}
	}
	.panel {
		flex: 0 0 100%;
		min-width: 0;
		height: 100%;
		min-height: 0;
		overflow: hidden;
		display: flex;
		flex-direction: column;
	}
	.dots {
		flex: none;
		display: flex;
		gap: 4px;
		height: 24px;
	}
	/* Small dots, each a 44 × 44 px target (the target reaches beyond the row's height). */
	.dots button {
		position: relative;
		display: grid;
		place-items: center;
		width: 44px;
		height: 24px;
		padding: 0;
		border: 0;
		background: transparent;
		cursor: pointer;
	}
	.dots button::after {
		content: '';
		position: absolute;
		inset: -10px 0;
	}
	.dots button span {
		width: 7px;
		height: 7px;
		border-radius: 4px;
		background: var(--fg-muted);
		opacity: 0.5;
		transition:
			width 0.2s,
			opacity 0.2s;
	}
	.dots button[aria-selected='true'] span {
		width: 18px;
		opacity: 1;
		background: var(--fg);
	}
	.dots button:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: -2px;
		border-radius: 999px;
	}
	@media (prefers-reduced-motion: reduce) {
		.dots button span {
			transition: none;
		}
	}
	@media (forced-colors: active) {
		.dots button span {
			background: ButtonText;
			opacity: 1;
		}
		.dots button[aria-selected='true'] span {
			background: Highlight;
		}
	}
</style>
