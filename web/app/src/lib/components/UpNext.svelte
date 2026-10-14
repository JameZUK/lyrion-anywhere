<script lang="ts">
	import { me } from '$lib/state/me';
	// "Up next" under the transport controls: one line naming the next track (Wave 3.5). A tap
	// opens the queue (the same sheet as the Queue button, through `onopen`, or else the queue
	// page); swipe the line sideways (or press Delete) to take that track out of the queue,
	// with Undo. Removing tracks otherwise lives in the queue itself.
	import { api, app } from '$lib/state/app.svelte';
	import { textOf } from '$lib/lms/format';
	import { isSwipe, removeChecked, restoreTrack } from '$lib/lms/queue-edit';
	import { upNext } from '$lib/lms/upnext';
	import type { Track } from '$lib/lms/types';
	import Snackbar from './Snackbar.svelte';
	import { tick } from 'svelte';

	let { onopen }: { onopen?: () => void } = $props();

	/** A swipe must travel this far (px) to remove. */
	const SWIPE_REMOVE_PX = 90;

	const next = $derived(upNext(app.now));
	/** Hidden at once when removed, until the next status says what follows now. */
	let removed = $state<string | null>(null);
	let undo = $state<{ track: Track; index: number; player: string } | null>(null);
	let notice = $state('');
	let swipe = $state<{ id: number; x: number; y: number; dx: number; active: boolean } | null>(null);
	let swallowClick = false;
	let box = $state<HTMLDivElement>();
	let snack = $state<ReturnType<typeof Snackbar>>();

	const key = (t: Track | undefined, i: number) => `${app.playerId}|${i}|${t?.id ?? t?.title ?? ''}`;
	const shown = $derived(next && key(next.track, next.index) !== removed ? next : null);
	const title = $derived(textOf(next?.track.title) || 'Next track');
	const artist = $derived(textOf(next?.track.artist));

	async function remove() {
		const n = shown;
		const player = app.playerId;
		if (!n || !player) return;
		// Removing hides the row: focus goes to the snackbar's Undo rather than being lost.
		const hadFocus = !!box?.contains(document.activeElement);
		removed = key(n.track, n.index);
		undo = null;
		notice = '';
		try {
			await removeChecked((p, c) => api.rpc(p, c), player, n.index, n.track, (p, e) => me.queueEdit(p, e));
			const canUndo = Number.isSafeInteger(n.track.id) && (n.track.id as number) > 0;
			undo = canUndo ? { track: n.track, index: n.index, player } : null;
			notice = `Removed ${textOf(n.track.title, 80) || 'track'}`;
			if (hadFocus) {
				await tick();
				snack?.focusAction();
			}
		} catch (e) {
			removed = null;
			notice = `Couldn't remove the track: ${(e as Error).message}`;
		}
		app.queueVersion++;
		app.soon();
	}

	async function restore() {
		const u = undo;
		dismiss();
		if (!u || u.player !== app.playerId) return;
		try {
			await restoreTrack((p, c) => api.rpc(p, c), u.player, u.track.id as number, u.index);
		} catch (e) {
			notice = `Couldn't put the track back: ${(e as Error).message}`;
		}
		removed = null;
		app.queueVersion++;
		app.soon();
	}

	function dismiss() {
		undo = null;
		notice = '';
	}

	function onkeydown(e: KeyboardEvent) {
		if (e.key === 'Delete' || e.key === 'Backspace') {
			e.preventDefault();
			void remove();
		}
	}

	// --- swipe sideways to remove (touch and pen; vertical movement stays a scroll) ---------

	function down(e: PointerEvent) {
		if (e.pointerType === 'mouse' || swipe) return;
		swipe = { id: e.pointerId, x: e.clientX, y: e.clientY, dx: 0, active: false };
	}
	function move(e: PointerEvent) {
		if (!swipe || e.pointerId !== swipe.id) return;
		const dx = e.clientX - swipe.x;
		const dy = e.clientY - swipe.y;
		if (!swipe.active) {
			if (!isSwipe(dx, dy)) {
				if (Math.abs(dy) > 12) swipe = null; // a scroll: not ours
				return;
			}
			(e.currentTarget as Element).setPointerCapture?.(e.pointerId);
		}
		swipe = { ...swipe, dx, active: true };
	}
	function up(e: PointerEvent, cancelled = false) {
		if (!swipe || e.pointerId !== swipe.id) return;
		const { dx, active } = swipe;
		swipe = null;
		if (!active) return;
		swallowClick = true;
		if (!cancelled && Math.abs(dx) >= SWIPE_REMOVE_PX) void remove();
	}
	function onclick(e: MouseEvent) {
		if (swallowClick) {
			swallowClick = false;
			e.preventDefault();
			return;
		}
		if (onopen) {
			e.preventDefault();
			onopen();
		}
	}
</script>

{#if shown}
	<div class="upnext" bind:this={box} data-up-next>
		<a
			class="row"
			href="#/queue"
			aria-haspopup={onopen ? 'dialog' : undefined}
			aria-label={`Up next: ${title}${artist ? ` by ${artist}` : ''}. Opens the queue; press Delete to remove it from the queue.`}
			style:transform={swipe?.active ? `translateX(${swipe.dx}px)` : undefined}
			style:opacity={swipe?.active ? Math.max(0.3, 1 - Math.abs(swipe.dx) / 240) : undefined}
			{onclick}
			{onkeydown}
			onpointerdown={down}
			onpointermove={move}
			onpointerup={(e) => up(e)}
			onpointercancel={(e) => up(e, true)}
		>
			<span class="kicker">Up next</span>
			<span class="line ellipsis">{title}{#if artist}<span class="muted">{` — ${artist}`}</span>{/if}</span>
			<span class="chev" aria-hidden="true">›</span>
		</a>
	</div>
{/if}
<Snackbar bind:this={snack} message={notice} action={undo ? 'Undo' : ''} onaction={restore} ondismiss={dismiss} />

<style>
	.upnext {
		max-width: 400px;
		width: 100%;
		margin-top: 0.25rem;
	}
	.row {
		display: flex;
		align-items: center;
		gap: 0.6rem;
		min-height: 44px;
		padding: 0 0.6rem;
		border-radius: var(--radius-sm);
		text-align: left;
		touch-action: pan-y;
		user-select: none;
	}
	.row:hover {
		background: color-mix(in srgb, var(--bg-raised) 55%, transparent);
	}
	.kicker {
		flex: none;
		font-size: 0.72rem;
		font-weight: 650;
		text-transform: uppercase;
		letter-spacing: 0.04em;
		color: var(--fg-muted);
	}
	.line {
		flex: 1;
		min-width: 0;
		font-size: 0.9rem;
	}
	.ellipsis {
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.chev {
		flex: none;
		color: var(--fg-muted);
	}
</style>
