<script lang="ts">
	// The A–Z fast scroller (Wave 2 §3.2), built from LMS's index: each letter jumps straight to
	// its offset. Drag along the strip (touch, pen or mouse) and a letter bubble shows where
	// letting go will land. For the keyboard and screen readers it is a toolbar with one tab
	// stop: the arrow keys, Home and End move between letters, and Enter or Space jumps.
	import { keyFromPointer, type IndexEntry } from '$lib/lms/azindex';

	let {
		index,
		onjump,
		noun = 'items',
	}: {
		index: readonly IndexEntry[];
		onjump: (e: IndexEntry) => void;
		/** What is listed, for labels ("albums"). */
		noun?: string;
	} = $props();

	let strip = $state<HTMLDivElement>();
	/** The letter with the tab stop. */
	let focusAt = $state(0);
	/** The letter shown in the bubble while dragging or moving with the keyboard, or null. */
	let bubble = $state<number | null>(null);
	let dragging: number | null = null;
	let swallowClick = false;
	let said = $state('');

	const buttons = () => [...(strip?.querySelectorAll<HTMLButtonElement>('button') ?? [])];
	const label = (e: IndexEntry) => `${e.key}: ${e.count.toLocaleString('en-GB')} ${noun}`;

	function jump(i: number) {
		const e = index[i];
		if (!e) return;
		focusAt = i;
		said = `Showing ${noun} from ${e.key}`;
		onjump(e);
	}

	function onkeydown(e: KeyboardEvent) {
		let to = -1;
		if (e.key === 'ArrowDown' || e.key === 'ArrowRight') to = Math.min(index.length - 1, focusAt + 1);
		else if (e.key === 'ArrowUp' || e.key === 'ArrowLeft') to = Math.max(0, focusAt - 1);
		else if (e.key === 'Home') to = 0;
		else if (e.key === 'End') to = index.length - 1;
		else if (e.key === 'PageDown') to = Math.min(index.length - 1, focusAt + 5);
		else if (e.key === 'PageUp') to = Math.max(0, focusAt - 5);
		if (to < 0) return;
		e.preventDefault();
		focusAt = to;
		bubble = to;
		buttons()[to]?.focus();
	}

	function at(e: PointerEvent): number {
		const r = strip?.getBoundingClientRect();
		return r ? keyFromPointer(index.length, r.top, r.height, e.clientY) : 0;
	}

	function down(e: PointerEvent) {
		if (e.pointerType === 'mouse' && e.button !== 0) return;
		e.preventDefault();
		strip?.setPointerCapture?.(e.pointerId);
		dragging = e.pointerId;
		bubble = at(e);
	}
	function move(e: PointerEvent) {
		if (dragging !== e.pointerId) return;
		bubble = at(e);
	}
	function up(e: PointerEvent, cancelled = false) {
		if (dragging !== e.pointerId) return;
		dragging = null;
		const i = bubble;
		bubble = null;
		if (cancelled || i === null) return;
		swallowClick = true;
		setTimeout(() => (swallowClick = false), 400);
		jump(i);
	}
	function click(i: number) {
		if (swallowClick) return;
		jump(i);
	}

	const shownKey = $derived(bubble === null ? undefined : index[bubble]?.key);

	$effect(() => {
		if (focusAt >= index.length) focusAt = 0;
	});
</script>

{#if index.length > 1}
	<div class="az">
		<div
			class="strip"
			role="toolbar"
			aria-label={`Jump to a letter in the ${noun}`}
			aria-orientation="vertical"
			tabindex="-1"
			bind:this={strip}
			{onkeydown}
			onpointerdown={down}
			onpointermove={move}
			onpointerup={(e) => up(e)}
			onpointercancel={(e) => up(e, true)}
			onfocusout={(e) => {
				if (!strip?.contains(e.relatedTarget as Node | null) && dragging === null) bubble = null;
			}}
			data-az
		>
			{#each index as e, i (e.offset)}
				<button
					type="button"
					tabindex={i === focusAt ? 0 : -1}
					aria-label={label(e)}
					onclick={() => click(i)}
					onfocus={() => (focusAt = i)}
				>
					<span aria-hidden="true">{e.key}</span>
				</button>
			{/each}
		</div>
		{#if shownKey}
			<div class="bubble" aria-hidden="true" data-az-bubble>{shownKey}</div>
		{/if}
		<p class="sr-only" role="status">{said}</p>
	</div>
{/if}

<style>
	.az {
		position: sticky;
		top: 4.5rem;
		float: right;
		z-index: 3;
		margin-left: 0.25rem;
		height: 0;
	}
	.strip {
		display: flex;
		flex-direction: column;
		align-items: center;
		width: 44px;
		max-height: calc(var(--app-h, 100vh) - var(--bar-h, 0px) - 7rem);
		overflow: hidden;
		padding: 0.25rem 0;
		border-radius: 999px;
		background: color-mix(in srgb, var(--bg-raised) 80%, transparent);
		/* The strip is a drag surface: dragging along it picks a letter, never scrolls. */
		touch-action: none;
		user-select: none;
		outline: none;
	}
	button {
		flex: 1 1 0;
		min-height: 14px;
		max-height: 24px;
		width: 44px;
		padding: 0;
		border: 0;
		background: transparent;
		color: var(--accent);
		font: inherit;
		font-size: 0.72rem;
		font-weight: 700;
		line-height: 1;
		cursor: pointer;
	}
	button:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: -2px;
		border-radius: 6px;
	}
	.bubble {
		position: absolute;
		right: 56px;
		top: 2rem;
		width: 64px;
		height: 64px;
		display: grid;
		place-items: center;
		border-radius: 50%;
		background: var(--accent);
		color: var(--bg);
		font-size: 2rem;
		font-weight: 700;
		box-shadow: var(--shadow);
		pointer-events: none;
	}
</style>
