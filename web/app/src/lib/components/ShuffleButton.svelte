<script lang="ts">
	// Album-aware shuffle (Wave 2 §3.5): pressing cycles Off → Tracks → Albums (LMS shuffle 0, 1,
	// 2), and the button's name says which. A long press, a right-click or the down arrow
	// offers the three as a menu.
	import { SHUFFLE_MODES, SHUFFLE_NAMES, nextShuffle, shuffleLabel, shuffleMode, type ShuffleMode } from '$lib/lms/shuffle';
	import Icon from './Icon.svelte';

	let { value, onchange, disabled = false }: { value: unknown; onchange: (m: ShuffleMode) => void; disabled?: boolean } = $props();

	/** How long a press must last to open the choices (ms). */
	const LONG_MS = 500;
	const mode = $derived(shuffleMode(value));
	let open = $state(false);
	let btn = $state<HTMLButtonElement>();
	let menu = $state<HTMLDivElement>();
	let timer: ReturnType<typeof setTimeout> | undefined;
	let longPressed = false;
	const uid = `shuffle-${Math.random().toString(36).slice(2, 9)}`;

	const items = () => [...(menu?.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]') ?? [])];
	function show() {
		open = true;
		queueMicrotask(() => items()[mode]?.focus());
	}
	function close(refocus = true) {
		open = false;
		if (refocus) btn?.focus();
	}
	function choose(m: ShuffleMode) {
		close();
		if (m !== mode) onchange(m);
	}

	function down(e: PointerEvent) {
		if (disabled || (e.pointerType === 'mouse' && e.button !== 0)) return;
		longPressed = false;
		clearTimeout(timer);
		timer = setTimeout(() => {
			longPressed = true;
			show();
		}, LONG_MS);
	}
	const cancel = () => clearTimeout(timer);
	function click() {
		cancel();
		if (longPressed) {
			longPressed = false;
			return;
		}
		onchange(nextShuffle(mode));
	}
	function keydown(e: KeyboardEvent) {
		if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
			e.preventDefault();
			show();
		}
	}
	function menuKey(e: KeyboardEvent) {
		const all = items();
		const at = all.indexOf(document.activeElement as HTMLButtonElement);
		let to = -1;
		if (e.key === 'ArrowDown') to = (at + 1) % all.length;
		else if (e.key === 'ArrowUp') to = (at - 1 + all.length) % all.length;
		else if (e.key === 'Home') to = 0;
		else if (e.key === 'End') to = all.length - 1;
		else if (e.key === 'Escape') {
			e.preventDefault();
			e.stopPropagation();
			close();
			return;
		} else if (e.key === 'Tab') {
			close(false);
			return;
		}
		if (to >= 0) {
			e.preventDefault();
			all[to]?.focus();
		}
	}
	$effect(() => {
		if (!open) return;
		const outside = (e: PointerEvent) => {
			const t = e.target as Node;
			if (!menu?.contains(t) && !btn?.contains(t)) close(false);
		};
		document.addEventListener('pointerdown', outside, true);
		return () => document.removeEventListener('pointerdown', outside, true);
	});
</script>

<span class="shuffle">
	<button
		bind:this={btn}
		id={uid}
		class="icon-btn"
		class:on={mode > 0}
		aria-label={shuffleLabel(mode)}
		aria-describedby={`${uid}-how`}
		aria-expanded={open ? true : undefined}
		aria-controls={open ? `${uid}-menu` : undefined}
		title={shuffleLabel(mode)}
		onclick={click}
		onkeydown={keydown}
		onpointerdown={down}
		onpointerup={cancel}
		onpointerleave={cancel}
		onpointercancel={cancel}
		oncontextmenu={(e) => {
			e.preventDefault();
			cancel();
			show();
		}}
		{disabled}
		data-shuffle={mode}
	>
		<Icon name="shuffle" />{#if mode === 2}<span class="mark" aria-hidden="true">A</span>{/if}
	</button>
	<span class="sr-only" id={`${uid}-how`}>Press to go from off to tracks to albums. Long press, or press the down arrow, to choose.</span>
	{#if open}
		<div class="menu" role="menu" id={`${uid}-menu`} aria-label="Shuffle" bind:this={menu} tabindex="-1" onkeydown={menuKey}>
			{#each SHUFFLE_MODES as m (m)}
				<button role="menuitemradio" aria-checked={m === mode} tabindex="-1" onclick={() => choose(m)} data-shuffle-choice={m}>
					<span class="tick" aria-hidden="true">{m === mode ? '✓' : ''}</span>{m === 0 ? 'Shuffle off' : `Shuffle ${SHUFFLE_NAMES[m].toLowerCase()}`}
				</button>
			{/each}
		</div>
	{/if}
</span>

<style>
	.shuffle {
		position: relative;
		display: inline-flex;
	}
	.icon-btn {
		position: relative;
	}
	.on {
		color: var(--accent);
	}
	.mark {
		position: absolute;
		right: 4px;
		bottom: 4px;
		font-size: 0.6rem;
		font-weight: 800;
		line-height: 1;
	}
	.menu {
		position: absolute;
		left: 0;
		bottom: 100%;
		z-index: 6;
		display: grid;
		min-width: 180px;
		padding: 0.35rem;
		background: var(--bg-raised);
		border: 1px solid var(--line);
		border-radius: var(--radius-sm);
		box-shadow: var(--shadow);
	}
	.menu button {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		min-height: 44px;
		padding: 0.5rem 0.7rem;
		border: 0;
		background: transparent;
		border-radius: 6px;
		text-align: left;
		color: var(--fg);
		font: inherit;
	}
	.menu button:hover,
	.menu button:focus-visible {
		background: var(--bg-sunken);
	}
	.tick {
		width: 1em;
		color: var(--accent);
	}
</style>
