<script lang="ts">
	import { me } from '$lib/state/me';
	// The selected player's queue. Edits (drag to reorder, swipe or tap to remove, Undo) show
	// at once and are then sent to LMS one at a time, in order; the next load reconciles. A
	// failed edit is undone locally and reported. Loads that land mid-gesture or mid-edit are
	// dropped (and repeated afterwards) so the 2-second refreshes never fight the user.
	//
	// Wave 2 §3.4: sections (Now playing, Up next — what this device added with Play next —
	// and Then), shown flat whenever the tracker can't vouch for them; swipe right to play
	// next and left to remove, each also in the row's menu and on keys (N, Delete).
	//
	// Wave 3.5: the header is the title and ⋯ (Save as playlist, Clear upcoming, Clear the
	// queue, each confirmed in a dialog); a row is its drag handle, the track (tap to play) and
	// ⋯ (Play next, Move up, Move down, Remove from queue). Gapless is shown in track info.
	import { tick, untrack } from 'svelte';
	import { api, app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import type { Command } from '$lib/api/rpc';
	import { duration, textOf } from '$lib/lms/format';
	import { CLEAR_UPCOMING_MAX, clearAfter, dropIndex, insertAt, isSwipe, moveChecked, playNextTarget, removeAt, removeChecked, reorder, restoreTrack, swipeAction, type RowBox, type ServerEdit } from '$lib/lms/queue-edit';
	import { rowShortcut } from '$lib/gestures/swipe';
	import { stateOf } from '$lib/queue/tracker';
	import { trackerVersion, upNextTracker } from '$lib/queue/upnext.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import RowMenu from '$lib/components/RowMenu.svelte';
	import type { PlayerStatus, Track } from '$lib/lms/types';
	import Art from '$lib/components/Art.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import Snackbar from '$lib/components/Snackbar.svelte';
	import SaveQueue from '$lib/components/SaveQueue.svelte';
	import type { SavedPlaylist } from '$lib/api/me';
	import { go } from '$lib/nav';
	import { haptics } from '$lib/haptics/haptics.svelte';
	import { controls } from '$lib/player/controls.svelte';
	import { MARK_LABELS } from '$lib/player/local-plan';
	import { isNetworkError } from '$lib/state/reachability';

	/** How long a swiped row takes to slide out before it is removed. */
	const SLIDE_MS = 180;
	/** Undo waits up to this long for LMS to add the track back. */
	const UNDO_POLLS = 10;
	const UNDO_POLL_MS = 150;

	let tracks = $state<Track[]>([]);
	let total = $state(0);
	let error = $state('');
	/** The queue has been read from the server at least once (for this player). */
	let loaded = $state(false);
	/** Why the last edit failed; kept until the next edit (a reload doesn't clear it). */
	let editError = $state('');
	/** Clear the queue's confirmation, while it is open. */
	let confirmClear = $state(false);
	let saver = $state<ReturnType<typeof SaveQueue>>();
	/** Clear upcoming's confirmation, while it is open. */
	let askUpcoming = $state(false);
	/** Clear upcoming's progress, for screen readers. */
	let clearing = $state('');
	let list = $state<HTMLDivElement>();

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
	/** The playlist the queue was just saved as (the message bar offers to open it). */
	let saved = $state<SavedPlaylist | null>(null);
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
			loaded = true;
			// Check Up next against the tracks LMS actually has there.
			upNextTracker.observe(id, stateOf(r));
			const cur = Number(r.playlist_cur_index ?? app.now?.playlist_cur_index ?? -1);
			if (Number.isInteger(cur) && cur >= 0) upNextTracker.verify(id, cur, tracks.map((t) => (typeof t.id === 'number' ? t.id : null)));
		} catch (e) {
			// Out of reach: the connection message says so; the queue as last read stays.
			error = isNetworkError(e) || app.reach !== 'online' ? '' : (e as Error).message;
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
	// Another player: its queue hasn't been read yet.
	$effect(() => {
		void app.playerId;
		untrack(() => (loaded = false));
	});

	// Another player: its queue is a different list, so an Undo for this one no longer applies.
	$effect(() => {
		const id = app.playerId;
		untrack(() => {
			if (undo && undo.player !== id) dismiss();
		});
	});

	const current = $derived(Number(app.now?.playlist_cur_index ?? -1));
	/** Which controls work now: out of reach, no edits, and only rows this device can play. */
	const g = $derived(controls.gates);
	/** Queue edits need the server. */
	const editable = $derived(app.reach === 'online');
	/** Up next, as tracked on this device; null shows the queue flat. */
	const sections = $derived.by(() => {
		void trackerVersion();
		return app.playerId && current >= 0 ? upNextTracker.sections(app.playerId, current, tracks.length) : null;
	});
	type Group = { key: string; title: string; start: number; end: number };
	/** The rows in groups: one untitled group when flat. */
	const groups = $derived.by((): Group[] => {
		const n = tracks.length;
		if (!sections || current >= n) return [{ key: 'all', title: '', start: 0, end: n - 1 }];
		const out: Group[] = [];
		if (current > 0) out.push({ key: 'played', title: 'Played', start: 0, end: current - 1 });
		out.push({ key: 'now', title: 'Now playing', start: current, end: current });
		out.push({ key: 'next', title: 'Up next', start: sections.from, end: sections.to });
		if (sections.to + 1 < n) out.push({ key: 'then', title: 'Then', start: sections.to + 1, end: n - 1 });
		return out;
	});
	const upcoming = $derived(current >= 0 ? Math.max(0, total - current - 1) : 0);

	/**
	 * Show `optimistic` now and send the commands (in order, after any earlier edit). On
	 * failure, put back what was shown before and say so. Resolves to whether it worked.
	 */
	function edit(optimistic: Track[], newTotal: number, run: (player: string) => Promise<unknown>, failure: string): Promise<boolean> {
		const player = app.playerId;
		if (!player || !editable) return Promise.resolve(false);
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
	const rpc = (p: string, c: Command) => api.rpc(p, c);

	async function remove(i: number) {
		const t = tracks[i];
		if (!t) return;
		dismiss();
		const player = app.playerId;
		// The removed row had focus (its Remove button): keep focus in the list, on the row
		// that takes its place (or the one before), rather than losing it to the page.
		const hadFocus = !!list?.contains(document.activeElement);
		const done = edit(removeAt(tracks, i), Math.max(0, total - 1), async (player) => {
			await removeChecked(rpc, player, i, t, serverEdit);
			upNextTracker.removed(player, i);
		}, "Couldn't remove the track");
		if (hadFocus) {
			await tick();
			const rows = list?.querySelectorAll<HTMLElement>('.main');
			(rows?.[i] ?? rows?.[i - 1])?.focus();
		}
		const ok = await done;
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
			async (player) => {
				const before = total - 1;
				await restoreTrack((p, c) => api.rpc(p, c), player, u.track.id as number, u.index, UNDO_POLLS, UNDO_POLL_MS);
				upNextTracker.appended(player);
				if (before > u.index) upNextTracker.moved(player, before, u.index);
			},
			"Couldn't put the track back",
		);
	}

	function dismiss() {
		undo = null;
		saved = null;
		notice = '';
	}

	/** The queue was saved as a playlist: say so, with a way to open it. */
	function savedAs(p: SavedPlaylist) {
		dismiss();
		saved = p;
		notice = `Saved as “${p.name}”`;
	}

	function openSaved() {
		const to = saved ? cmd.link('playlist', saved.id) : null;
		dismiss();
		if (to) void go(to);
	}

	async function move(from: number, to: number) {
		if (from === to || from < 0 || to < 0 || from >= tracks.length || to >= tracks.length) return;
		const t = tracks[from]!;
		await edit(reorder(tracks, from, to), total, async (player) => {
			await moveChecked(rpc, player, from, to, t, serverEdit);
			upNextTracker.moved(player, from, to);
		}, "Couldn't move the track");
	}

	/** Play the track at `i` next: move it to just after the current one (it joins Up next). */
	async function playNext(i: number) {
		const t = tracks[i];
		const player = app.playerId;
		if (!t || !player || current < 0) return;
		dismiss();
		if (i === current) {
			notice = `${textOf(t.title, 80) || 'That track'} is playing now`;
			return;
		}
		const to = playNextTarget(i, current);
		if (to === i) upNextTracker.moved(player, i, i);
		else await move(i, to);
		if (!editError) notice = `${textOf(t.title, 80) || 'Track'} will play next`;
	}

	/** Clear upcoming: every track after the current one, from the end, each one checked. */
	async function clearUpcoming() {
		askUpcoming = false;
		const player = app.playerId;
		if (!player || current < 0) return;
		dismiss();
		const cur = current;
		const shown = tracks;
		clearing = 'Clearing upcoming tracks…';
		const ok = await edit(shown.slice(0, cur + 1), cur + 1, async (p) => {
			await clearAfter(rpc, p, shown, cur, (n) => (clearing = `Removed ${n} of ${shown.length - cur - 1}`), serverEdit);
			upNextTracker.truncated(p, cur + 1);
		}, "Couldn't clear upcoming");
		clearing = ok ? 'Cleared upcoming tracks' : '';
	}

	async function clearAll() {
		askUpcoming = false;
		dismiss();
		const player = app.playerId;
		await edit([], 0, send(cmd.clearQueue()), "Couldn't clear the queue");
		if (player) upNextTracker.truncated(player, 0);
	}

	async function clear() {
		confirmClear = false;
		dismiss();
		const player = app.playerId;
		await edit([], 0, send(cmd.clearQueue()), "Couldn't clear the queue");
		if (player) upNextTracker.truncated(player, 0);
	}

	// --- keyboard: Alt+ArrowUp/Down moves the focused row ---------------------------------

	async function onRowKey(e: KeyboardEvent, i: number) {
		if (rowShortcut(e, { next: () => void playNext(i), remove: () => void remove(i) })) return;
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
		return [...(list?.querySelectorAll('li[data-index]') ?? [])].map((li) => {
			const r = li.getBoundingClientRect();
			return { top: r.top, height: r.height };
		});
	}

	function dragStart(e: PointerEvent, i: number) {
		if (drag || leaving !== null || !editable || (e.pointerType === 'mouse' && e.button !== 0)) return;
		e.preventDefault();
		(e.currentTarget as Element).setPointerCapture?.(e.pointerId);
		drag = { from: i, to: i, pointerId: e.pointerId, startY: e.clientY, dy: 0, rows: rowBoxes() };
		haptics.tick('pickup');
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
		haptics.tick('drop');
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
		if (e.pointerType === 'mouse' || drag || swipe || leaving !== null || !editable) return;
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
		swipe = { ...swipe, dx };
	}

	function swipeEnd(e: PointerEvent, cancelled = false) {
		if (!swipe || e.pointerId !== swipe.pointerId) return;
		const s = swipe;
		swipe = null;
		if (!s.active) return;
		swallowClick = true;
		setTimeout(() => (swallowClick = false), 400);
		const way = cancelled ? null : swipeAction(s.dx, s.width);
		if (way === 'right') {
			void playNext(s.index);
			settle();
			return;
		}
		if (way !== 'left') {
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
		if (swallowClick || !g.jump(i)) return;
		controls.jump(i);
	}

	function rowShift(i: number): string | undefined {
		if (drag && i === drag.from) return `translateY(${drag.dy}px)`;
		if (swipe?.active && i === swipe.index) return `translateX(${swipe.dx}px)`;
		return undefined;
	}
	/** The gateway's checked edits (check and change in one step), where it has them. */
	const serverEdit: ServerEdit = (p, e) => me.queueEdit(p, e);
</script>

{#snippet row(t: Track, i: number)}
	{@const mark = controls.mark(i, t.id)}
	{@const playable = g.jump(i)}
	<li
		data-index={i}
		class:now={i === current}
		class:unavailable={!playable}
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
			aria-label={`Play ${textOf(t.title)}${mark ? `. ${MARK_LABELS[mark]}` : ''}`}
			aria-current={i === current ? 'true' : undefined}
			disabled={!playable}
			data-mark={mark ?? undefined}
		>
			<div class="thumb"><Art path={cmd.coverPath(t.coverid ?? t.artwork_track_id, 96)} alt="" label={textOf(t.title)} /></div>
			<span class="text">
				<span class="title ellipsis">{textOf(t.title)}</span>
				<span class="sub ellipsis muted">{[textOf(t.artist), textOf(t.album)].filter(Boolean).join(' · ')}</span>
			</span>
			{#if mark}<span class="mark" title={MARK_LABELS[mark]} data-queue-mark={mark}><Icon name={mark === 'downloaded' ? 'downloaded' : 'device'} size={15} /></span>{/if}
			<span class="dur muted">{duration(t.duration)}</span>
		</button>
		<!-- One trailing control (Wave 3.5 rule 3): everything else is in its menu. -->
		<RowMenu
			label={`More options for ${textOf(t.title) || 'this track'}`}
			actions={[
				{ label: 'Play next', icon: 'playnext', run: () => void playNext(i), shortcut: 'N', disabled: !editable, why: controls.why(!editable) },
				{ label: 'Move up', icon: 'up', run: () => void move(i, i - 1), shortcut: 'Alt+ArrowUp', disabled: i === 0 || !editable, why: controls.why(!editable) },
				{ label: 'Move down', icon: 'down', run: () => void move(i, i + 1), shortcut: 'Alt+ArrowDown', disabled: i === tracks.length - 1 || !editable, why: controls.why(!editable) },
				{ label: 'Remove from queue', icon: 'trash', run: () => void remove(i), shortcut: 'Delete', disabled: !editable, why: controls.why(!editable), danger: true },
			]}
		/>
	</li>
{/snippet}

<section class="page">
	<header>
		<div>
			<h1>Queue</h1>
			<p class="muted">{textOf(app.current?.name)}{total ? ` · ${total} tracks` : ''}</p>
		</div>
		{#if tracks.length}
			<RowMenu
				label="Queue options"
				actions={[
					...(saver?.available() ? [{ label: 'Save as playlist…', icon: 'plus', run: () => saver?.start(), disabled: !editable, why: controls.why(!editable), id: 'save-queue' }] : []),
					...(upcoming > 0 ? [{ label: 'Clear upcoming…', icon: 'trash', run: () => (askUpcoming = true), disabled: !editable, why: controls.why(!editable), danger: true, id: 'clear-upcoming' }] : []),
					{ label: 'Clear the queue…', icon: 'trash', run: () => (confirmClear = true), disabled: !editable, why: controls.why(!editable), danger: true, id: 'clear' },
				]}
				trigger={{ 'data-queue-more': true }}
			/>
		{/if}
		<SaveQueue bind:this={saver} button={false} {tracks} onsaved={savedAs} />
	</header>
	{#if editError}<p class="error" role="alert">{editError}</p>{/if}
	{#if error}<p class="error">{error}</p>{/if}
	{#if !app.playerId}
		<p class="muted">No player chosen. Choose a player with Play on to see its queue.</p>
	{:else if !tracks.length && !loaded && app.reach !== 'online'}
		<!-- Never "empty" when it simply couldn't be read. -->
		<p class="muted" data-queue-unloaded>The queue can’t be loaded until your music server is back.</p>
	{:else if !tracks.length && loaded}
		<p class="muted">The queue is empty. <a class="link" href="#/library">Find something to play</a></p>
	{:else if !tracks.length}
		<p class="muted">Loading…</p>
	{:else}
		<p class="sr-only">To reorder, focus a track and press Alt with the up or down arrow, or use Move up and Move down in its menu. Press N to play a track next, or Delete to remove it.</p>
		{#if clearing}<p class="sr-only" role="status">{clearing}</p>{/if}
		<div class="q" class:dragging={drag !== null} bind:this={list} data-sections={sections ? 'on' : 'off'}>
			{#each groups as g (g.key)}
				{#if g.title}<h2 class="section" id={`q-${g.key}`}>{g.title}</h2>{/if}
				<ol start={g.start + 1} aria-labelledby={g.title ? `q-${g.key}` : undefined} data-group={g.key}>
					{#each tracks.slice(g.start, g.end + 1) as t, k (`${g.start + k}:${t.id}`)}
						{@render row(t, g.start + k)}
					{/each}
				</ol>
			{/each}
		</div>
		{#if total > tracks.length}<p class="muted">Showing the first {tracks.length} of {total}.</p>{/if}
	{/if}
</section>

{#if confirmClear}
	<ConfirmDialog
		title="Clear the queue?"
		message={`Take out all ${total === 1 ? 'the track' : `${total.toLocaleString('en-GB')} tracks`}? Playback stops.`}
		confirmLabel="Clear"
		danger
		onconfirm={clear}
		oncancel={() => (confirmClear = false)}
	/>
{/if}
{#if askUpcoming}
	{#if upcoming > CLEAR_UPCOMING_MAX || upcoming > tracks.length - current - 1}
		<ConfirmDialog
			title="Clear the whole queue?"
			message={`There are ${upcoming.toLocaleString('en-GB')} tracks after this one: taking them out one at a time would take a while. Clear the whole queue instead? Playback stops.`}
			confirmLabel="Clear all"
			danger
			onconfirm={clearAll}
			oncancel={() => (askUpcoming = false)}
		/>
	{:else}
		<ConfirmDialog
			title="Clear upcoming?"
			message={`Take out the ${upcoming === 1 ? 'track' : `${upcoming} tracks`} after the one playing now?`}
			confirmLabel="Clear upcoming"
			danger
			onconfirm={clearUpcoming}
			oncancel={() => (askUpcoming = false)}
		/>
	{/if}
{/if}

<Snackbar message={notice} action={undo ? 'Undo' : saved ? 'Open' : ''} onaction={undo ? restore : openSaved} ondismiss={dismiss} />

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
	.q ol {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	.section {
		margin: 1rem 0 0.35rem;
		font-size: 0.8rem;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--fg-muted);
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
		width: 36px;
		height: 44px;
		color: var(--fg-muted);
		cursor: grab;
		/* The handle starts a drag, never a scroll. */
		touch-action: none;
	}
	@media (pointer: coarse) {
		.handle {
			width: 44px;
		}
	}
	.dragging .handle {
		cursor: grabbing;
	}
	/* Out of reach: rows this device can't play are dimmed (and their buttons disabled). */
	li.unavailable .main {
		opacity: 0.45;
		cursor: not-allowed;
	}
	.mark {
		flex: none;
		display: inline-grid;
		place-items: center;
		color: var(--accent);
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
	.link {
		text-decoration: underline;
	}
	/* Phones: no room for the length. */
	@media (max-width: 600px) {
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
