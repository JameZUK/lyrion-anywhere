<script lang="ts">
	// The selected player's queue. Edits (drag to reorder, swipe or tap to remove, Undo) show
	// at once and are then sent to LMS one at a time, in order; the next load reconciles. A
	// failed edit is undone locally and reported. Loads that land mid-gesture or mid-edit are
	// dropped (and repeated afterwards) so the 2-second refreshes never fight the user.
	import { tick, untrack } from 'svelte';
	import { api, app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import type { Command } from '$lib/api/rpc';
	import { duration, textOf } from '$lib/lms/format';
	import { dropIndex, insertAt, isSwipe, removeAt, reorder, restoreTrack, swipeRemoves, type RowBox } from '$lib/lms/queue-edit';
	import type { PlayerStatus, Track } from '$lib/lms/types';
	import Art from '$lib/components/Art.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import Snackbar from '$lib/components/Snackbar.svelte';

	/** How long a swiped row takes to slide out before it is removed. */
	const SLIDE_MS = 180;
	/** Undo waits up to this long for LMS to add the track back. */
	const UNDO_POLLS = 10;
	const UNDO_POLL_MS = 150;

	let tracks = $state<Track[]>([]);
	let total = $state(0);
	let error = $state('');
	/** Why the last edit failed; kept until the next edit (a reload doesn't clear it). */
	let editError = $state('');
	let confirmClear = $state(false);
	let list = $state<HTMLOListElement>();

	type Drag = { from: number; to: number; pointerId: number; startY: number; dy: number; rows: RowBox[] };
	type Swipe = { index: number; pointerId: number; startX: number; startY: number; dx: number; active: boolean; width: number };
	let drag = $state<Drag | null>(null);
	let swipe = $state<Swipe | null>(null);
	/** The row sliding out after a swipe. */
	let leaving = $state<number | null>(null);
	/** Set after a swipe so the tap that ends it doesn't also play the track. */
	let swallowClick = false;

	type Undo = { track: Track; index: number; player: string };
	let undo = $state<Undo | null>(null);
	let notice = $state('');

	// Edits in flight, and a counter bumped by each: a load started before an edit is stale.
	let pending = 0;
	let editSeq = 0;
	let stale = false;
	let chain: Promise<unknown> = Promise.resolve();

	const busy = () => drag !== null || swipe?.active === true || leaving !== null || pending > 0;

	async function load() {
		const id = app.playerId;
		if (!id) return;
		const seq = editSeq;
		try {
			const r = await api.rpc<PlayerStatus>(id, cmd.queue(0, 500));
			if (id !== app.playerId) return;
			if (busy() || seq !== editSeq) {
				stale = true; // load again once the gesture or edit is over
				return;
			}
			tracks = r.playlist_loop ?? [];
			total = r.playlist_tracks ?? tracks.length;
			error = '';
		} catch (e) {
			error = (e as Error).message;
		}
	}

	function settle() {
		if (stale && !busy()) {
			stale = false;
			void load();
		}
	}

	$effect(() => {
		void app.playerId;
		void app.queueVersion;
		untrack(() => void load());
	});

	// Another player: its queue is a different list, so an Undo for this one no longer applies.
	$effect(() => {
		const id = app.playerId;
		untrack(() => {
			if (undo && undo.player !== id) dismiss();
		});
	});

	const current = $derived(Number(app.now?.playlist_cur_index ?? -1));

	/**
	 * Show `optimistic` now and send the commands (in order, after any earlier edit). On
	 * failure, put back what was shown before and say so. Resolves to whether it worked.
	 */
	function edit(optimistic: Track[], newTotal: number, run: (player: string) => Promise<unknown>, failure: string): Promise<boolean> {
		const player = app.playerId;
		if (!player) return Promise.resolve(false);
		const before = tracks;
		const beforeTotal = total;
		tracks = optimistic;
		total = newTotal;
		editError = '';
		editSeq++;
		pending++;
		const done = chain.then(async () => {
			try {
				await run(player);
				return true;
			} catch (e) {
				if (player === app.playerId) {
					tracks = before;
					total = beforeTotal;
					editError = `${failure}: ${(e as Error).message}`;
				}
				return false;
			} finally {
				pending--;
				editSeq++;
				stale = true;
				settle();
				app.soon();
			}
		});
		chain = done;
		return done;
	}

	const send = (c: Command) => (player: string) => api.rpc(player, c);

	async function remove(i: number) {
		const t = tracks[i];
		if (!t) return;
		dismiss();
		const player = app.playerId;
		const ok = await edit(removeAt(tracks, i), Math.max(0, total - 1), send(cmd.removeFromQueue(i)), "Couldn't remove the track");
		if (!ok) return;
		const canUndo = Number.isSafeInteger(t.id) && (t.id as number) > 0;
		undo = canUndo ? { track: t, index: i, player } : null;
		notice = `Removed ${textOf(t.title, 80) || 'track'}`;
	}

	/** Put a removed track back where it was: add it to the end, then move it into place. */
	async function restore() {
		const u = undo;
		dismiss();
		if (!u || u.player !== app.playerId) return;
		await edit(
			insertAt(tracks, u.index, u.track),
			total + 1,
			(player) => restoreTrack((p, c) => api.rpc(p, c), player, u.track.id as number, u.index, UNDO_POLLS, UNDO_POLL_MS),
			"Couldn't put the track back",
		);
	}

	function dismiss() {
		undo = null;
		notice = '';
	}

	async function move(from: number, to: number) {
		if (from === to || from < 0 || to < 0 || from >= tracks.length || to >= tracks.length) return;
		await edit(reorder(tracks, from, to), total, send(cmd.moveInQueue(from, to)), "Couldn't move the track");
	}

	async function clear() {
		if (!confirmClear) {
			confirmClear = true;
			setTimeout(() => (confirmClear = false), 4000);
			return;
		}
		confirmClear = false;
		dismiss();
		await edit([], 0, send(cmd.clearQueue()), "Couldn't clear the queue");
	}

	// --- keyboard: Alt+ArrowUp/Down moves the focused row ---------------------------------

	async function onRowKey(e: KeyboardEvent, i: number) {
		if (!e.altKey || (e.key !== 'ArrowUp' && e.key !== 'ArrowDown')) return;
		e.preventDefault();
		const to = i + (e.key === 'ArrowUp' ? -1 : 1);
		if (to < 0 || to >= tracks.length) return;
		const done = move(i, to);
		await tick();
		list?.querySelectorAll<HTMLElement>('.main')[to]?.focus();
		await done;
	}

	// --- drag to reorder, from the handle (mouse, pen or touch) ----------------------------

	function rowBoxes(): RowBox[] {
		return [...(list?.children ?? [])].map((li) => {
			const r = li.getBoundingClientRect();
			return { top: r.top, height: r.height };
		});
	}

	function dragStart(e: PointerEvent, i: number) {
		if (drag || leaving !== null || (e.pointerType === 'mouse' && e.button !== 0)) return;
		e.preventDefault();
		(e.currentTarget as Element).setPointerCapture?.(e.pointerId);
		drag = { from: i, to: i, pointerId: e.pointerId, startY: e.clientY, dy: 0, rows: rowBoxes() };
	}

	function dragMove(e: PointerEvent) {
		if (!drag || e.pointerId !== drag.pointerId) return;
		e.preventDefault();
		const dy = e.clientY - drag.startY;
		const r = drag.rows[drag.from];
		const centre = r ? r.top + r.height / 2 + dy : e.clientY;
		drag = { ...drag, dy, to: dropIndex(drag.rows, drag.from, centre) };
	}

	function dragEnd(e: PointerEvent, cancelled = false) {
		if (!drag || e.pointerId !== drag.pointerId) return;
		const { from, to } = drag;
		drag = null;
		if (!cancelled && from !== to) void move(from, to);
		else settle();
	}

	/** Where the drop marker goes: above or below which row. */
	function dropMark(i: number): 'before' | 'after' | '' {
		if (!drag || drag.from === drag.to || i === drag.from) return '';
		if (drag.to < drag.from) return i === drag.to ? 'before' : '';
		return i === drag.to ? 'after' : '';
	}

	// --- swipe left to remove (touch only; vertical movement stays a scroll) ----------------

	function swipeStart(e: PointerEvent, i: number) {
		if (e.pointerType === 'mouse' || drag || swipe || leaving !== null) return;
		const li = (e.currentTarget as Element).closest('li');
		swipe = { index: i, pointerId: e.pointerId, startX: e.clientX, startY: e.clientY, dx: 0, active: false, width: li?.getBoundingClientRect().width ?? 0 };
	}

	function swipeMove(e: PointerEvent) {
		if (!swipe || e.pointerId !== swipe.pointerId) return;
		const dx = e.clientX - swipe.startX;
		const dy = e.clientY - swipe.startY;
		if (!swipe.active) {
			if (!isSwipe(dx, dy)) return;
			swipe = { ...swipe, active: true };
		}
		swipe = { ...swipe, dx: Math.min(0, dx) };
	}

	function swipeEnd(e: PointerEvent, cancelled = false) {
		if (!swipe || e.pointerId !== swipe.pointerId) return;
		const s = swipe;
		swipe = null;
		if (!s.active) return;
		swallowClick = true;
		setTimeout(() => (swallowClick = false), 400);
		if (cancelled || !swipeRemoves(s.dx, s.width)) {
			settle();
			return;
		}
		leaving = s.index;
		setTimeout(() => {
			leaving = null;
			void remove(s.index);
		}, SLIDE_MS);
	}

	function play(i: number) {
		if (swallowClick) return;
		void app.send(cmd.jump(i));
	}

	function rowShift(i: number): string | undefined {
		if (drag && i === drag.from) return `translateY(${drag.dy}px)`;
		if (swipe?.active && i === swipe.index) return `translateX(${swipe.dx}px)`;
		return undefined;
	}
</script>

<section class="page">
	<header>
		<div>
			<h1>Queue</h1>
			<p class="muted">{textOf(app.current?.name)}{total ? ` · ${total} tracks` : ''}</p>
		</div>
		{#if tracks.length}
			<button class="btn" class:danger={confirmClear} onclick={clear}><Icon name="trash" size={18} />{confirmClear ? 'Tap again to clear' : 'Clear'}</button>
		{/if}
	</header>
	{#if editError}<p class="error" role="alert">{editError}</p>{/if}
	{#if error}<p class="error">{error}</p>{/if}
	{#if !app.playerId}
		<p class="muted">Choose a player to see its queue.</p>
	{:else if !tracks.length}
		<p class="muted">The queue is empty. <a class="link" href="#/albums">Find something to play</a></p>
	{:else}
		<p class="sr-only">To reorder, focus a track and press Alt with the up or down arrow, or use the move buttons.</p>
		<ol class="q" class:dragging={drag !== null} bind:this={list}>
			{#each tracks as t, i (`${i}:${t.id}`)}
				<li
					data-index={i}
					class:now={i === current}
					class:lifted={drag?.from === i}
					class:swiping={swipe?.active && swipe.index === i}
					class:leaving={leaving === i}
					class:drop-before={dropMark(i) === 'before'}
					class:drop-after={dropMark(i) === 'after'}
					style:transform={rowShift(i)}
				>
					<span
						class="handle"
						aria-hidden="true"
						title="Drag to reorder"
						onpointerdown={(e) => dragStart(e, i)}
						onpointermove={dragMove}
						onpointerup={(e) => dragEnd(e)}
						onpointercancel={(e) => dragEnd(e, true)}
						onlostpointercapture={(e) => dragEnd(e, true)}
					>
						<svg width="16" height="16" viewBox="0 0 24 24"><path d="M9 6h.01M15 6h.01M9 12h.01M15 12h.01M9 18h.01M15 18h.01" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" /></svg>
					</span>
					<button
						class="main"
						onclick={() => play(i)}
						onkeydown={(e) => onRowKey(e, i)}
						onpointerdown={(e) => swipeStart(e, i)}
						onpointermove={swipeMove}
						onpointerup={(e) => swipeEnd(e)}
						onpointercancel={(e) => swipeEnd(e, true)}
						aria-label={`Play ${textOf(t.title)}`}
						aria-current={i === current ? 'true' : undefined}
					>
						<div class="thumb"><Art path={cmd.coverPath(t.coverid ?? t.artwork_track_id, 96)} alt="" label={textOf(t.title)} /></div>
						<span class="text">
							<span class="title ellipsis">{textOf(t.title)}</span>
							<span class="sub ellipsis muted">{[textOf(t.artist), textOf(t.album)].filter(Boolean).join(' · ')}</span>
						</span>
						<span class="dur muted">{duration(t.duration)}</span>
					</button>
					<div class="ops">
						<button class="icon-btn mv" aria-label={`Move ${textOf(t.title)} up`} onclick={() => move(i, i - 1)} onkeydown={(e) => onRowKey(e, i)} disabled={i === 0}><Icon name="back" size={16} /></button>
						<button class="icon-btn mv" aria-label={`Move ${textOf(t.title)} down`} onclick={() => move(i, i + 1)} onkeydown={(e) => onRowKey(e, i)} disabled={i === tracks.length - 1}><Icon name="chevron" size={16} /></button>
						<button class="icon-btn" aria-label={`Remove ${textOf(t.title)}`} onclick={() => remove(i)}><Icon name="close" size={16} /></button>
					</div>
				</li>
			{/each}
		</ol>
		{#if total > tracks.length}<p class="muted">Showing the first {tracks.length} of {total}.</p>{/if}
	{/if}
</section>

{#if notice}
	<Snackbar message={notice} action={undo ? 'Undo' : ''} onaction={restore} ondismiss={dismiss} />
{/if}

<style>
	header {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: 1rem;
		margin-bottom: 1rem;
	}
	h1,
	header p {
		margin: 0;
	}
	.danger {
		border-color: var(--danger);
		color: var(--danger);
	}
	.q {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	.q.dragging {
		user-select: none;
	}
	li {
		position: relative;
		display: flex;
		align-items: center;
		border-radius: var(--radius-sm);
		background: var(--bg);
		/* Vertical movement is the browser's (scrolling); sideways is ours (swipe to remove). */
		touch-action: pan-y;
		transition: transform 0.18s ease;
	}
	li:hover {
		background: var(--bg-sunken);
	}
	li.now {
		background: var(--accent-soft);
	}
	li.now .title {
		color: var(--accent);
	}
	li.lifted,
	li.swiping {
		transition: none;
		z-index: 2;
	}
	li.lifted {
		box-shadow: var(--shadow);
		background: var(--bg-raised);
	}
	li.leaving {
		transform: translateX(-110%);
		opacity: 0;
		transition:
			transform 0.18s ease-in,
			opacity 0.18s ease-in;
	}
	li.drop-before::before,
	li.drop-after::after {
		content: '';
		position: absolute;
		left: 0;
		right: 0;
		height: 3px;
		border-radius: 2px;
		background: var(--accent);
	}
	li.drop-before::before {
		top: -2px;
	}
	li.drop-after::after {
		bottom: -2px;
	}
	.handle {
		flex: none;
		display: grid;
		place-items: center;
		width: 28px;
		height: 44px;
		color: var(--fg-muted);
		cursor: grab;
		/* The handle starts a drag, never a scroll. */
		touch-action: none;
	}
	.dragging .handle {
		cursor: grabbing;
	}
	.main {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 0.8rem;
		padding: 0.45rem 0.6rem 0.45rem 0.2rem;
		border: 0;
		background: transparent;
		text-align: left;
	}
	.thumb {
		width: 44px;
		flex: none;
	}
	.text {
		flex: 1;
		min-width: 0;
		display: grid;
	}
	.title {
		font-weight: 550;
	}
	.sub {
		font-size: 0.85rem;
	}
	.dur {
		font-size: 0.85rem;
		font-variant-numeric: tabular-nums;
	}
	.ops {
		display: flex;
		opacity: 0.55;
	}
	li:hover .ops,
	.ops:focus-within {
		opacity: 1;
	}
	.ops .mv:first-child :global(svg) {
		transform: rotate(90deg);
	}
	.link {
		text-decoration: underline;
	}
	/* Phones: the move buttons stay in the tab order for keyboard and screen-reader users, but
	   are only shown while one of them has focus (drag the handle instead). */
	@media (max-width: 600px) {
		.ops:not(:focus-within) .mv {
			position: absolute;
			width: 1px;
			height: 1px;
			overflow: hidden;
			clip: rect(0 0 0 0);
			white-space: nowrap;
		}
		.dur {
			display: none;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		li,
		li.leaving {
			transition: none;
		}
	}
</style>
