<script lang="ts">
	import type { Track } from '$lib/lms/types';
	import type { TrackBadge } from './track-badge';
	import { duration, textOf } from '$lib/lms/format';
	import Icon from './Icon.svelte';

	let {
		tracks,
		onplay,
		onnext,
		onadd,
		onmore,
		showArtist = false,
		badge,
	}: {
		tracks: Track[];
		onplay: (t: Track, index: number) => void;
		onnext?: (t: Track) => void;
		onadd?: (t: Track) => void;
		/** "More…": the track's context menu (info, plugin actions). */
		onmore?: (t: Track) => void;
		showArtist?: boolean;
		/** A per-track status mark, e.g. whether it is downloaded to this device. */
		badge?: (t: Track) => TrackBadge | undefined;
	} = $props();

	// Each row's "More options" is a menu button (WAI-ARIA APG): Enter, Space or the arrow keys
	// open it with focus on an item; arrows, Home and End move; Escape closes it and puts focus
	// back on the button; Tab or a click elsewhere closes it.
	let open = $state<number | null>(null);
	let menuEl = $state<HTMLDivElement>();
	const uid = Math.random().toString(36).slice(2, 9);
	const triggers: HTMLButtonElement[] = [];

	const items = () => [...(menuEl?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? [])];

	function openMenu(i: number, focus: 'first' | 'last' = 'first') {
		open = i;
		queueMicrotask(() => {
			const all = items();
			(focus === 'last' ? all.at(-1) : all[0])?.focus();
		});
	}

	function closeMenu(refocus = true) {
		const i = open;
		open = null;
		if (refocus && i !== null) triggers[i]?.focus();
	}

	function triggerKey(e: KeyboardEvent, i: number) {
		if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
			e.preventDefault();
			openMenu(i, e.key === 'ArrowUp' ? 'last' : 'first');
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
			closeMenu();
			return;
		} else if (e.key === 'Tab') {
			closeMenu(false);
			return;
		}
		if (to >= 0) {
			e.preventDefault();
			all[to]?.focus();
		}
	}

	function choose(action: () => void) {
		action();
		closeMenu();
	}

	// A click or tap anywhere else closes the menu.
	$effect(() => {
		if (open === null) return;
		const outside = (e: PointerEvent) => {
			const t = e.target as Node;
			if (!menuEl?.contains(t) && !triggers[open!]?.contains(t)) closeMenu(false);
		};
		document.addEventListener('pointerdown', outside, true);
		return () => document.removeEventListener('pointerdown', outside, true);
	});
</script>

<ol class="tracks">
	{#each tracks as t, i (t.id ?? i)}
		<li class="row">
			<button class="main" onclick={() => onplay(t, i)} aria-label={`Play ${textOf(t.title)}`}>
				<span class="num muted">{textOf(t.tracknum) || i + 1}</span>
				<span class="text">
					<span class="title ellipsis">{textOf(t.title)}</span>
					{#if showArtist && t.artist}<span class="artist ellipsis muted">{textOf(t.artist)}</span>{/if}
				</span>
				{#if badge}
					{@const b = badge(t)}
					{#if b}<span class="badge" class:done={b.kind === 'done'} class:failed={b.kind === 'failed'} title={b.label}><Icon name={b.kind === 'failed' ? 'close' : b.kind === 'done' ? 'check' : 'download'} size={16} label={b.label} /></span>{/if}
				{/if}
				<span class="dur muted">{duration(t.duration)}</span>
			</button>
			{#if onnext || onadd || onmore}
				<div class="more">
					<button
						class="icon-btn"
						bind:this={triggers[i]}
						id={`tl-${uid}-${i}`}
						aria-label={`More options for ${textOf(t.title) || 'this track'}`}
						aria-haspopup="menu"
						aria-expanded={open === i}
						aria-controls={open === i ? `tl-${uid}-${i}-menu` : undefined}
						onclick={() => (open === i ? closeMenu() : openMenu(i))}
						onkeydown={(e) => triggerKey(e, i)}
						data-track-more
					>
						<Icon name="more" size={20} />
					</button>
					{#if open === i}
						<div class="menu" role="menu" id={`tl-${uid}-${i}-menu`} aria-labelledby={`tl-${uid}-${i}`} bind:this={menuEl} tabindex="-1" onkeydown={menuKey}>
							{#if onnext}<button role="menuitem" tabindex="-1" onclick={() => choose(() => onnext(t))}><Icon name="playnext" size={18} />Play next</button>{/if}
							{#if onadd}<button role="menuitem" tabindex="-1" onclick={() => choose(() => onadd(t))}><Icon name="plus" size={18} />Add to queue</button>{/if}
							{#if onmore}<button role="menuitem" tabindex="-1" onclick={() => choose(() => onmore(t))}><Icon name="more" size={18} />More…</button>{/if}
						</div>
					{/if}
				</div>
			{/if}
		</li>
	{/each}
</ol>

<style>
	.tracks {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	.row {
		display: flex;
		align-items: center;
		border-radius: var(--radius-sm);
	}
	.row:hover {
		background: var(--bg-sunken);
	}
	.main {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 0.9rem;
		padding: 0.6rem 0.6rem;
		border: 0;
		background: transparent;
		text-align: left;
	}
	.num {
		width: 2ch;
		text-align: right;
		font-variant-numeric: tabular-nums;
	}
	.text {
		flex: 1;
		min-width: 0;
		display: grid;
	}
	.artist {
		font-size: 0.85rem;
	}
	.badge {
		display: grid;
		place-items: center;
		color: var(--fg-muted);
	}
	.badge.done {
		color: var(--accent);
	}
	.badge.failed {
		color: var(--danger);
	}
	.dur {
		font-variant-numeric: tabular-nums;
		font-size: 0.9rem;
	}
	.more {
		position: relative;
	}
	.menu {
		position: absolute;
		right: 0;
		top: 100%;
		z-index: 5;
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
		gap: 0.6rem;
		min-height: 44px;
		padding: 0.55rem 0.7rem;
		border: 0;
		background: transparent;
		border-radius: 6px;
		text-align: left;
	}
	.menu button:hover,
	.menu button:focus-visible {
		background: var(--bg-sunken);
	}
</style>
